# AMM Price Function Challenge

Design custom price functions for an AMM. Your goal: maximize **edge**.

## Submission

Upload a compiled BPF `.so` file to the submission API. Build it with:

```bash
cargo build-sbf --manifest-path programs/my-strategy/Cargo.toml
# Upload: programs/my-strategy/target/deploy/my_strategy.so
```

Local results may diverge slightly from submission scores due to different RNG seeds. Run more simulations locally (`--simulations 1000`) to reduce variance.

## The Simulation

Each simulation runs 10,000 steps. At each step:

1. **Price moves** — A fair price `p` evolves via geometric Brownian motion
2. **Arbitrageurs trade** — They push each AMM's spot price toward `p`, extracting profit
3. **Retail orders arrive** — Random buy/sell orders get routed optimally across AMMs

Your program competes against a **normalizer AMM** running a constant-product curve with 30 bps fees. Both AMMs start with identical reserves (100 X, 10,000 Y at price 100).

### Price Process

The fair price follows GBM: `S(t+1) = S(t) · exp(-σ²/2 + σZ)` where `Z ~ N(0,1)`

- Drift `μ = 0` (no directional bias)
- Per-step volatility `σ ~ U[0.088%, 0.101%]` (varies across simulations)

### Retail Flow

Uninformed traders arrive via Poisson process:

- Arrival rate `λ ~ U[0.6, 1.0]` orders per step
- Order size `~ LogNormal(μ, σ=1.2)` with mean `~ U[19, 21]` in Y terms
- Direction: 50% buy, 50% sell

Retail flow splits optimally between AMMs based on pricing — better prices attract more volume.

## The Math

### Program Interface

Your program receives **25 bytes of instruction data**:

| Offset | Size | Field        | Type | Description                  |
|--------|------|--------------|------|------------------------------|
| 0      | 1    | side         | u8   | 0=buy X (Y input), 1=sell X |
| 1      | 8    | input_amount | u64  | Input token amount (1e9)     |
| 9      | 8    | reserve_x    | u64  | Current X reserve (1e9)      |
| 17     | 8    | reserve_y    | u64  | Current Y reserve (1e9)      |

Return 8 bytes via `sol_set_return_data` — the `output_amount: u64` (1e9 scale).

Programs are stateless. Reserves are passed on each call and updated by the engine after execution.

### Arbitrage

When spot price diverges from fair price `p`, arbitrageurs binary-search for the optimal trade size — the point where marginal profit hits zero. Convexity of the pricing function guarantees convergence.

Higher effective fees mean arbitrageurs need larger mispricings to profit, so your AMM stays "stale" longer — bad for edge. But too-high fees push retail flow to the normalizer.

### Order Routing

Retail orders split optimally across AMMs via grid search over split ratio α ∈ [0, 1]. The router picks the α that maximizes total output for the trader.

Better pricing → more flow. But the relationship is nonlinear — small pricing differences can shift large fractions of volume.

### Edge

Edge measures profitability using the fair price at trade time:

```
Edge = Σ (amount_y - amount_x × fair_price)   for sells (AMM sells X, gets Y)
     + Σ (amount_x × fair_price - amount_y)   for buys  (AMM buys X, pays Y)
```

- **Retail trades**: Positive edge (you profit from the spread)
- **Arbitrage trades**: Negative edge (you lose to informed flow)

Good strategies maximize retail edge while minimizing arb losses.

## Why the Normalizer?

Without competition, setting 10% fees would appear profitable — you'd capture huge spreads on the few trades that still execute. The normalizer prevents this: if your pricing is too aggressive, retail routes to the 30 bps AMM and you get nothing.

The normalizer also means there's no "free lunch" — you can't beat 30 bps just by setting 29 bps. The optimal pricing depends on market conditions.

## Writing a Program

**Start with `programs/starter/`** — a constant-product AMM with 50 bps fees:

```bash
cp -r programs/starter programs/my-strategy
# Edit programs/my-strategy/Cargo.toml — change the package name
# Edit programs/my-strategy/src/lib.rs — change the swap logic
```

The starter implements `compute_swap` — pure math that takes instruction data and returns an output amount. The pinocchio entrypoint and FFI export are boilerplate:

```rust
use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey, _accounts: &[AccountInfo], instruction_data: &[u8],
) -> ProgramResult {
    let output = compute_swap(instruction_data);
    unsafe { pinocchio::syscalls::sol_set_return_data(output.to_le_bytes().as_ptr(), 8); }
    Ok(())
}

/// This is where your logic goes.
pub fn compute_swap(data: &[u8]) -> u64 {
    // Decode inputs
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as u128;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as u128;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as u128;

    // Your pricing logic here...
    todo!()
}

/// Required for native execution (local testing).
#[cfg(not(target_os = "solana"))]
#[no_mangle]
pub unsafe extern "C" fn compute_swap_ffi(data: *const u8, len: usize) -> u64 {
    compute_swap(core::slice::from_raw_parts(data, len))
}
```

### Requirements

| Requirement | Description |
|-------------|-------------|
| **Convex** | Marginal price must worsen with trade size. Non-convex programs are rejected. |
| **Monotonic** | Larger input must produce larger output. |
| **Stateless** | Reserves are passed each call. No state between calls. |
| **< 100k CU** | Must execute within the compute unit limit. |

### Tips

- Use `u128` intermediates to avoid overflow (reserves at 1e9 scale can multiply to ~1e24)
- Test convexity with `prop-amm validate` before running simulations
- Think about how your marginal price schedule affects the routing split
- The arbitrageur is efficient — don't try to extract value from informed flow

## CLI

```bash
# Build everything (native + BPF)
prop-amm build programs/my-strategy

# Run 1000 simulations (~10 seconds)
prop-amm run programs/my-strategy/target/release/libmy_strategy.dylib

# Quick test
prop-amm run programs/my-strategy/target/release/libmy_strategy.dylib --simulations 10

# Validate BPF program (convexity, monotonicity, CU)
prop-amm validate programs/my-strategy/target/deploy/my_strategy.so
```

Output is your average edge across simulations. The 30 bps normalizer typically scores around 250-350 edge per simulation depending on market conditions.

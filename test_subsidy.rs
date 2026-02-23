use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};
use prop_amm_submission_sdk::{set_return_data_bytes, set_return_data_u64};

const NAME: &str = "Subsidy Test";
const MODEL_USED: &str = "None";
const STORAGE_SIZE: usize = 1024;

// Parameters to optimize
const BASE_FEE_BPS: u128 = 40;
const SUBSIDY_BPS: u128 = 100;
const SUBSIDY_CAP_Y: u128 = 20_000_000; // 0.02 Y in nano

#[derive(wincode::SchemaRead)]
struct ComputeSwapInstruction {
    side: u8,
    input_amount: u64,
    reserve_x: u64,
    reserve_y: u64,
    _storage: [u8; STORAGE_SIZE],
}

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if instruction_data.is_empty() {
        return Ok(());
    }
    match instruction_data[0] {
        0 | 1 => {
            let output = compute_swap(instruction_data);
            set_return_data_u64(output);
        }
        2 => {}
        3 => set_return_data_bytes(NAME.as_bytes()),
        4 => set_return_data_bytes(get_model_used().as_bytes()),
        _ => {}
    }
    Ok(())
}

pub fn get_model_used() -> &'static str {
    MODEL_USED
}

fn cp_out(side: u8, input: u128, rx: u128, ry: u128, gamma_num: u128) -> u128 {
    if input == 0 || rx == 0 || ry == 0 {
        return 0;
    }
    let net = input.saturating_mul(gamma_num) / 10_000;
    let k = rx.saturating_mul(ry);
    match side {
        0 => {
            let new_ry = ry.saturating_add(net);
            rx.saturating_sub((k + new_ry - 1) / new_ry)
        }
        1 => {
            let new_rx = rx.saturating_add(net);
            ry.saturating_sub((k + new_rx - 1) / new_rx)
        }
        _ => 0,
    }
}

pub fn compute_swap(data: &[u8]) -> u64 {
    let decoded: ComputeSwapInstruction = match wincode::deserialize(data) {
        Ok(decoded) => decoded,
        Err(_) => return 0,
    };

    let side = decoded.side;
    let input = decoded.input_amount as u128;
    let rx = decoded.reserve_x as u128;
    let ry = decoded.reserve_y as u128;

    if input == 0 || rx == 0 || ry == 0 {
        return 0;
    }

    let base = cp_out(side, input, rx, ry, 10_000u128.saturating_sub(BASE_FEE_BPS));
    
    let bonus = match side {
        0 => {
            // Buy X: bonus in X terms
            let cap_x = SUBSIDY_CAP_Y.saturating_mul(rx) / ry.max(1);
            let num = input.saturating_mul(rx).saturating_mul(SUBSIDY_BPS);
            let den = ry.saturating_mul(10_000).max(1);
            (num / den).min(cap_x)
        }
        1 => {
            // Sell X: bonus in Y terms
            let num = input.saturating_mul(ry).saturating_mul(SUBSIDY_BPS);
            let den = rx.saturating_mul(10_000).max(1);
            (num / den).min(SUBSIDY_CAP_Y)
        }
        _ => 0,
    };
    
    let out = base.saturating_add(bonus);
    let capped = match side {
        0 => out.min(rx.saturating_sub(1)),
        1 => out.min(ry.saturating_sub(1)),
        _ => 0,
    };
    
    capped as u64
}

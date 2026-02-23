use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};
use prop_amm_submission_sdk::{set_return_data_bytes, set_return_data_u64};

const NAME: &str = "Fee Test";
const MODEL_USED: &str = "None";
const STORAGE_SIZE: usize = 1024;

// Test different fee levels - change this to test
const FEE_BPS: u128 = 150; // 30 basis points
const FEE_NUMERATOR: u128 = 10000 - FEE_BPS;
const FEE_DENOMINATOR: u128 = 10000;

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

pub fn compute_swap(data: &[u8]) -> u64 {
    let decoded: ComputeSwapInstruction = match wincode::deserialize(data) {
        Ok(decoded) => decoded,
        Err(_) => return 0,
    };

    let side = decoded.side;
    let input_amount = decoded.input_amount as u128;
    let reserve_x = decoded.reserve_x as u128;
    let reserve_y = decoded.reserve_y as u128;

    if reserve_x == 0 || reserve_y == 0 {
        return 0;
    }

    let k = reserve_x * reserve_y;

    match side {
        0 => {
            let net_y = input_amount * FEE_NUMERATOR / FEE_DENOMINATOR;
            let new_ry = reserve_y + net_y;
            let k_div = (k + new_ry - 1) / new_ry;
            reserve_x.saturating_sub(k_div) as u64
        }
        1 => {
            let net_x = input_amount * FEE_NUMERATOR / FEE_DENOMINATOR;
            let new_rx = reserve_x + net_x;
            let k_div = (k + new_rx - 1) / new_rx;
            reserve_y.saturating_sub(k_div) as u64
        }
        _ => 0,
    }
}

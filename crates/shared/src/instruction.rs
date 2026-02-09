/// Instruction data layout (25 bytes):
/// | Offset | Size | Field        | Type | Description                    |
/// |--------|------|--------------|------|--------------------------------|
/// | 0      | 1    | side         | u8   | 0=buy X (Y input), 1=sell X   |
/// | 1      | 8    | input_amount | u64  | Input token amount (1e9 scale) |
/// | 9      | 8    | reserve_x    | u64  | Current X reserve (1e9 scale)  |
/// | 17     | 8    | reserve_y    | u64  | Current Y reserve (1e9 scale)  |

pub const INSTRUCTION_SIZE: usize = 25;

pub fn encode_instruction(side: u8, input_amount: u64, reserve_x: u64, reserve_y: u64) -> [u8; INSTRUCTION_SIZE] {
    let mut data = [0u8; INSTRUCTION_SIZE];
    data[0] = side;
    data[1..9].copy_from_slice(&input_amount.to_le_bytes());
    data[9..17].copy_from_slice(&reserve_x.to_le_bytes());
    data[17..25].copy_from_slice(&reserve_y.to_le_bytes());
    data
}

pub fn decode_instruction(data: &[u8]) -> (u8, u64, u64, u64) {
    let side = data[0];
    let input_amount = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let reserve_x = u64::from_le_bytes(data[9..17].try_into().unwrap());
    let reserve_y = u64::from_le_bytes(data[17..25].try_into().unwrap());
    (side, input_amount, reserve_x, reserve_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let side = 1u8;
        let amount = 123_456_789_000u64;
        let rx = 100_000_000_000u64;
        let ry = 10_000_000_000_000u64;

        let encoded = encode_instruction(side, amount, rx, ry);
        let (s, a, x, y) = decode_instruction(&encoded);

        assert_eq!(s, side);
        assert_eq!(a, amount);
        assert_eq!(x, rx);
        assert_eq!(y, ry);
    }
}

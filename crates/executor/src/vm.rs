use solana_rbpf::{
    aligned_memory::AlignedMemory,
    ebpf,
    memory_region::{MemoryMapping, MemoryRegion},
    vm::EbpfVm,
};

use crate::loader::{BpfProgram, ExecutorError};
use crate::syscalls::SyscallContext;
use prop_amm_shared::instruction::INSTRUCTION_SIZE;

/// Solana input buffer layout for 0 accounts:
/// [0..8]   u64 num_accounts = 0
/// [8..16]  u64 instruction_data_len = INSTRUCTION_SIZE
/// [16..41] instruction_data (25 bytes)
/// [41..73] program_id (32 bytes, zeros)
const INPUT_BUF_SIZE: usize = 8 + 8 + INSTRUCTION_SIZE + 32;

pub struct BpfExecutor {
    program: BpfProgram,
    input_buf: Vec<u8>,
    stack: AlignedMemory<{ ebpf::HOST_ALIGN }>,
    heap: AlignedMemory<{ ebpf::HOST_ALIGN }>,
}

impl BpfExecutor {
    pub fn new(program: BpfProgram) -> Self {
        let config = program.executable().get_config();
        let mut input_buf = vec![0u8; INPUT_BUF_SIZE];
        input_buf[8..16].copy_from_slice(&(INSTRUCTION_SIZE as u64).to_le_bytes());

        Self {
            stack: AlignedMemory::zero_filled(config.stack_size()),
            heap: AlignedMemory::zero_filled(32 * 1024),
            program,
            input_buf,
        }
    }

    pub fn execute(
        &mut self,
        side: u8,
        amount: u64,
        rx: u64,
        ry: u64,
    ) -> Result<u64, ExecutorError> {
        // Write instruction data into the Solana-format input buffer
        self.input_buf[16] = side;
        self.input_buf[17..25].copy_from_slice(&amount.to_le_bytes());
        self.input_buf[25..33].copy_from_slice(&rx.to_le_bytes());
        self.input_buf[33..41].copy_from_slice(&ry.to_le_bytes());

        // Zero the stack for each call
        self.stack.as_slice_mut().fill(0);

        let executable = self.program.executable();
        let loader = self.program.loader();
        let config = executable.get_config();
        let sbpf_version = executable.get_sbpf_version();
        let stack_len = self.stack.len();

        let regions: Vec<MemoryRegion> = vec![
            executable.get_ro_region(),
            MemoryRegion::new_writable(self.stack.as_slice_mut(), ebpf::MM_STACK_START),
            MemoryRegion::new_writable(self.heap.as_slice_mut(), ebpf::MM_HEAP_START),
            MemoryRegion::new_writable(&mut self.input_buf, ebpf::MM_INPUT_START),
        ];

        let memory_mapping = MemoryMapping::new(regions, config, sbpf_version)
            .map_err(|e| ExecutorError::Execution(e.to_string()))?;

        let mut context = SyscallContext::new(100_000);

        let mut vm = EbpfVm::new(
            loader.clone(),
            sbpf_version,
            &mut context,
            memory_mapping,
            stack_len,
        );

        // Use JIT when available (x86_64), fall back to interpreter
        let use_interpreter = !self.program.jit_available();
        let (_instruction_count, result) = vm.execute_program(executable, use_interpreter);

        let result: Result<u64, _> = result.into();
        result.map_err(|e| ExecutorError::Execution(e.to_string()))?;

        if !context.has_return_data {
            return Err(ExecutorError::NoReturnData);
        }

        Ok(u64::from_le_bytes(context.return_data))
    }
}

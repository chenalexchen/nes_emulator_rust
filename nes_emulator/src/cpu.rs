/**
 * Simulator of 6502.
 *
 * For 6502 instruction references, see http://www.obelisk.me.uk/6502/reference.html and http://www.6502.org/tutorials/6502opcodes.html
 */
use simple_error::SimpleError;
use std::collections::HashMap;
use std::result::Result;
use bitflags::bitflags;

// NES platform has a special mechanism to mark where the CPU should start the execution.
// Upon inserting a new cartridge, the CPU receives a special signal called "Reset interrupt"
// that instructs CPU to set pc to 0xfffc.
const INIT_PROGRAM_COUNTER_ADDR: u16 = 0xfffc;

// Memory layout.

// Max address.
const MEM_ADDR_MAX: u16 = 0xffff;
const MEM_ADDR_SPACE_SIZE: usize = MEM_ADDR_MAX as usize + 1;
// Program ROM address.
const MEM_PRG_ROM_ADDR_START: u16 = 0x8000;
const MEM_PRG_ROM_ADDR_END: u16 = 0xffff;
const MEM_PRG_ROM_SIZE: usize = (MEM_PRG_ROM_ADDR_END - MEM_PRG_ROM_ADDR_START) as usize + 1;

const DEBUG_ADDR: u16 = 0xffff;

// Represents a 6502 CPU opcodes.
struct OpCode {
    pub code: u8,
    pub name: &'static str,
    pub bytes: u8,
    pub cycles: u8,
    pub addressing_mode: AddressingMode,
}

impl OpCode {
    fn new(
        code: u8,
        name: &'static str,
        bytes: u8,
        cycles: u8,
        addressing_mode: AddressingMode,
    ) -> Self {
        OpCode {
            code: code,
            name: name,
            bytes: bytes,
            cycles: cycles,
            addressing_mode: addressing_mode,
        }
    }
}

const OPCODE_BRK: u8 = 0x00;
const OPCODE_LDA_IMMEDIATE: u8 = 0xa9;
const OPCODE_LDA_ZEROPAGE: u8 = 0xa5;
const OPCODE_LDA_ZEROPAGEX: u8 = 0xb5;
const OPCODE_LDA_ABSOLUTE: u8 = 0xad;
const OPCODE_LDA_ABSOLUTEX: u8 = 0xbd;
const OPCODE_LDA_ABSOLUTEY: u8 = 0xb9;
const OPCODE_LDA_INDIRECTX: u8 = 0xa1;
const OPCODE_LDA_INDIRECTY: u8 = 0xb1;
const OPCODE_JMP_ABSOLUTE: u8 = 0x4c;
const OPCODE_JMP_INDIRECT: u8 = 0x6c;
const OPCODE_INX: u8 = 0xe8;
const OPCODE_TAX: u8 = 0xaa;

lazy_static! {
    // Hardcoded 6502 instructions.
    static ref OPCODES : Vec<OpCode> = vec![
        OpCode::new(OPCODE_BRK, "BRK", 0, 7, AddressingMode::NoneAddressing),

        OpCode::new(OPCODE_JMP_ABSOLUTE, "JMP", 3, 3, AddressingMode::Absolute),

        OpCode::new(OPCODE_LDA_IMMEDIATE, "LDA", 2, 2, AddressingMode::Immediate),
        OpCode::new(OPCODE_LDA_ZEROPAGE, "LDA", 2, 2, AddressingMode::ZeroPage),
        OpCode::new(OPCODE_LDA_ZEROPAGEX, "LDA", 2, 2, AddressingMode::ZeroPageX),
        OpCode::new(OPCODE_LDA_ABSOLUTE, "LDA", 3, 4, AddressingMode::Absolute),
        // Cycles +1 if page crossed.
        OpCode::new(OPCODE_LDA_ABSOLUTEX, "LDA", 3, 4, AddressingMode::AbsoluteX),
        // Cycles +1 if page crossed.
        OpCode::new(OPCODE_LDA_ABSOLUTEY, "LDA", 3, 4, AddressingMode::AbsoluteY),
        OpCode::new(OPCODE_LDA_INDIRECTX, "LDA", 2, 6, AddressingMode::IndirectX),
        // Cycles +1 if page crossed.
        OpCode::new(OPCODE_LDA_INDIRECTY, "LDA", 2, 5, AddressingMode::IndirectY),

        OpCode::new(OPCODE_INX, "INX", 1, 2, AddressingMode::NoneAddressing),

        OpCode::new(OPCODE_TAX, "TAX", 1, 1, AddressingMode::NoneAddressing),
    ];
    static ref OPCODE_MAP: HashMap<u8, &'static OpCode> = {
        let mut map: HashMap<u8, &'static OpCode> = HashMap::new();
        for opcode in &*OPCODES {
            map.insert(opcode.code, opcode);
        }
        map
    };
}

// Represents the memory of 6502.
struct Mem {
    // The maximum addressable memory is 64KB.
    data: [u8; MEM_ADDR_SPACE_SIZE],
}

impl Mem {
    pub fn new() -> Self {
        Mem {
            data: [0; MEM_ADDR_SPACE_SIZE],
        }
    }
    pub fn read(&self, addr: u16) -> u8 {
        self.data[addr as usize]
    }

    // Reads two bytes starting at |addr|. Little endian.
    pub fn read16(&self, addr: u16) -> Result<u16, SimpleError> {
        if addr == MEM_ADDR_MAX {
            return Err(SimpleError::new(format!(
                "cannot read two bytes starting from address 0x{:x}",
                MEM_ADDR_MAX
            )));
        }

        let lo = self.read(addr) as u16;
        let hi = self.read(addr.wrapping_add(1)) as u16;

        Ok((hi << 8) | lo)
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        self.data[addr as usize] = val;
    }

    pub fn write16(&mut self, addr: u16, val: u16) -> Result<(), SimpleError> {
        if addr == MEM_ADDR_MAX {
            return Err(SimpleError::new(format!(
                "cannot write two bytes at address 0x{:x}",
                MEM_ADDR_MAX
            )));
        }

        let lo = val as u8;
        self.write(addr, lo);

        let hi = (val >> 8) as u8;
        self.write(addr.wrapping_add(1), hi);

        Ok(())
    }

    pub fn write_range(&mut self, start_addr: u16, val: &[u8]) -> Result<(), SimpleError> {
        if start_addr as usize + val.len() > self.data.len() {
            return Err(SimpleError::new(format!(
                "Range exceeds the memory space: start_addr = 0x{:x}, range_length = {}",
                start_addr,
                val.len()
            )));
        }

        for i in 0..val.len() {
            self.write(start_addr + (i as u16), val[i]);
        }
        Ok(())
    }
}

#[derive(Debug)]
enum AddressingMode {
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    Indirect,
    IndirectX,
    IndirectY,
    NoneAddressing,
}

type InstructionHandler = fn(&mut CPU, u16);

lazy_static! {
    static ref INSTRUCTION_HANDLERS: HashMap<u8, InstructionHandler> = {
        let mut map: HashMap<u8, InstructionHandler> = HashMap::new();

        map.insert(OPCODE_BRK, CPU::brk);

        map.insert(OPCODE_LDA_IMMEDIATE, CPU::lda);
        map.insert(OPCODE_LDA_ZEROPAGE, CPU::lda);
        map.insert(OPCODE_LDA_ZEROPAGEX, CPU::lda);
        map.insert(OPCODE_LDA_ABSOLUTE, CPU::lda);
        map.insert(OPCODE_LDA_ABSOLUTEX, CPU::lda);
        map.insert(OPCODE_LDA_ABSOLUTEY, CPU::lda);
        map.insert(OPCODE_LDA_INDIRECTX, CPU::lda);
        map.insert(OPCODE_LDA_INDIRECTY, CPU::lda);

        map.insert(OPCODE_JMP_ABSOLUTE, CPU::jmp);

        map.insert(OPCODE_INX, CPU::inx);

        map.insert(OPCODE_TAX, CPU::tax);

        map
    };
}

// Status register.
// Note that we only have 7 status registers for 8 bits of "process status" register.
// Bit 5 is always set to 1. Since nothing can change it, it is of no use to programmers.
//
// See https://www.atarimagazines.com/compute/issue53/047_1_All_About_The_Status_Register.php
bitflags! {
    pub struct Status : u8 {
        const C = 0b0000_0001;   // C bit: bit 0
        const Z = 0b0000_0010;   // Z bit: bit 1
        const I = 0b0000_0100;   // I bit: bit 2
        const D = 0b0000_1000;   // D bit: bit 3
        const B = 0b0001_0000;   // B bit: bit 4
        const V = 0b0100_0000;   // V bit: bit 6
        const N = 0b1000_0000;   // N bit: bit 7
    }
}

pub struct CPU {
    pub reg_a: u8,      // register A.
    pub reg_x: u8,      // register X.
    pub reg_y: u8,      // register Y.
    pub reg_status: Status, // program status register.
    pub pc: u16,        // program counter.
    mem: Mem,           // Memory.
}

impl CPU {
    pub fn new() -> Self {
        CPU {
            reg_a: 0,
            reg_x: 0,
            reg_y: 0,
            reg_status: Status::empty(),
            pc: 0,
            mem: Mem::new(),
        }
    }

    // Loads the program into PRG ROM.
    pub fn load(&mut self, program: &[u8]) -> Result<(), SimpleError> {
        self.mem.write_range(MEM_PRG_ROM_ADDR_START, program)?;
        self.mem
            .write16(INIT_PROGRAM_COUNTER_ADDR, MEM_PRG_ROM_ADDR_START)
    }

    // NES platform has a special mechanism to mark where the CPU should start the execution. Upon inserting a new cartridge, the CPU receives a special signal called "Reset interrupt" that instructs CPU to:
    // 1) reset the state (registers and flags);
    // 2) set program_counter to the 16-bit address that is stored at 0xFFFC.
    pub fn reset(&mut self) {
        self.reg_a = 0;
        self.reg_x = 0;
        self.reg_y = 0;
        self.reg_status = Status::empty();
        self.pc = self.mem.read16(INIT_PROGRAM_COUNTER_ADDR).unwrap();
    }

    // Runs the program started at PRG ROM.
    pub fn run(&mut self) {
        self.pc = MEM_PRG_ROM_ADDR_START;

        loop {
            if !self.step() {
                break;
            }
        }
    }

    // Executes the next instruction, return true to continue.
    pub fn step(&mut self) -> bool {
        let val = self.read_mem(self.pc);
        match OPCODE_MAP.get(&val) {
            Some(opcode) => return self.dispatch_instruction(opcode),
            None => {
                panic!(todo!("opcode not yet implemented"));
            }
        }
    }

    pub fn interpret(&mut self, program: &[u8]) -> Result<(), SimpleError> {
        self.load(program)?;
        self.reset();
        self.run();

        Ok(())
    }

    fn dispatch_instruction(&mut self, opcode: &OpCode) -> bool {
        let curr_pc = self.pc;
        let handler = INSTRUCTION_HANDLERS.get(&opcode.code).unwrap();
        let addr = self.read_operand_address(self.pc.wrapping_add(1), &opcode.addressing_mode);
        handler(self, addr);

        // Advance program counters if no jump happens.
        if curr_pc == self.pc {
            self.pc = self.pc.wrapping_add(opcode.bytes as u16);
        }

        if opcode.code == OPCODE_BRK {
            false // stop
        } else {
            true // continue
        }
    }

    fn read_operand_address(&self, addr: u16, addr_mode: &AddressingMode) -> u16 {
        match addr_mode {
            AddressingMode::Immediate => addr,

            AddressingMode::ZeroPage => self.read_mem(addr) as u16,

            AddressingMode::ZeroPageX => self.read_mem(addr).wrapping_add(self.reg_x) as u16,

            AddressingMode::ZeroPageY => self.read_mem(addr).wrapping_add(self.reg_y) as u16,

            AddressingMode::Absolute => self.read_mem16(addr),

            AddressingMode::AbsoluteX => self.read_mem16(addr).wrapping_add(self.reg_x as u16),

            AddressingMode::AbsoluteY => self.read_mem16(addr).wrapping_add(self.reg_y as u16),

            AddressingMode::Indirect => {
                let addr_of_addr = self.read_mem16(addr);
                self.read_mem16(addr_of_addr)
            }

            AddressingMode::IndirectX => {
                let addr = self.read_mem(addr).wrapping_add(self.reg_x) as u16;
                self.read_mem16(addr)
            }

            AddressingMode::IndirectY => {
                let addr = self.read_mem16(addr);
                self.read_mem16(addr).wrapping_add(self.reg_y as u16)
            }

            AddressingMode::NoneAddressing => {
                // This address returned should never be used.
                DEBUG_ADDR
            }
        }
    }

    fn read_mem(&self, addr: u16) -> u8 {
        self.mem.read(addr)
    }

    fn read_mem16(&self, addr: u16) -> u16 {
        self.mem.read16(addr).unwrap()
    }

    fn write_mem(&mut self, addr: u16, val: u8) {
        self.mem.write(addr, val)
    }

    fn write_mem16(&mut self, addr: u16, val: u16) {
        self.mem.write16(addr, val).unwrap()
    }

    fn write_range(&mut self, start_addr: u16, val: &[u8]) {
        self.mem.write_range(start_addr, val).unwrap()
    }

    // Sets the N bit of status register based on the value of |register|.
    fn set_negative_flag(&mut self, register: u8) {
        if register & 0b1000_0000 == 0 {
            self.reg_status.remove(Status::N);
        } else {
            self.reg_status.insert(Status::N);
        }
    }

    // Sets the Z bit of status register based on the value of |register|.
    fn set_zero_flag(&mut self, register: u8) {
        if register == 0 {
            self.reg_status.insert(Status::Z);
        } else {
            self.reg_status.remove(Status::Z);
        }
    }

    // Handles instruction LDA.
    fn brk(&mut self, _addr: u16) {}

    // Handles instruction INX.
    fn inx(&mut self, _addr: u16) {
        let (val_x, _overflow) = self.reg_x.overflowing_add(1);
        self.reg_x = val_x;

        self.set_negative_flag(self.reg_x);
        self.set_zero_flag(self.reg_x);
    }

    fn jmp(&mut self, addr: u16) {
        self.pc = addr;
    }

    // Handles instruction LDA.
    fn lda(&mut self, addr: u16) {
        self.reg_a = self.read_mem(addr);

        self.set_negative_flag(self.reg_a);
        self.set_zero_flag(self.reg_a);
    }

    // Handles instruction TAX.
    fn tax(&mut self, _addr: u16) {
        self.reg_x = self.reg_a;

        self.set_negative_flag(self.reg_x);
        self.set_zero_flag(self.reg_x);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_mem_init() {
        let mem = Mem::new();

        for i in 0..0xffff {
            assert_eq!(mem.read(i as u16), 0x00);
        }
    }

    #[test]
    fn test_mem_read_write() {
        let mut mem = Mem::new();

        mem.write(0x01, 0xff);

        assert_eq!(mem.read(0x01), 0xff);
    }

    #[test]
    fn test_mem_read16() {
        let mut mem = Mem::new();

        mem.write(0x01, 0xff);
        mem.write(0x02, 0xcc);

        assert_eq!(mem.read16(0x01), Ok(0xccff));
    }

    #[test]
    fn test_mem_read16_out_of_range() {
        let mem = Mem::new();

        assert_eq!(
            mem.read16(0xffff),
            Err(SimpleError::new(
                "cannot read two bytes starting from address 0xffff"
            ))
        );
    }

    #[test]
    fn test_mem_write_range() {
        let mut mem = Mem::new();
        let input: Vec<u8> = vec![0, 1, 2, 3, 4, 5];

        assert_eq!(mem.write_range(0x01, &input[1..]), Ok(()));

        assert_eq!(mem.read(0x01), 1);
        assert_eq!(mem.read(0x02), 2);
        assert_eq!(mem.read(0x03), 3);
        assert_eq!(mem.read(0x04), 4);
        assert_eq!(mem.read(0x05), 5);
    }

    #[test]
    fn test_mem_write16() {
        let mut mem = Mem::new();

        assert_eq!(mem.write16(0x01, 0xffcc), Ok(()));

        assert_eq!(mem.read16(0x01), Ok(0xffcc));
    }

    #[test]
    fn test_mem_write16_out_or_range() {
        let mut mem = Mem::new();

        assert_eq!(
            mem.write16(0xffff, 0xffff),
            Err(SimpleError::new("cannot write two bytes at address 0xffff"))
        );
    }

    #[test]
    fn test_mem_write_range_out_of_range() {
        let mut mem = Mem::new();
        let input: Vec<u8> = vec![0, 1, 2, 3, 4, 5];

        assert_eq!(
            mem.write_range(0xfffe, &input[1..]),
            Err(SimpleError::new(
                "Range exceeds the memory space: start_addr = 0xfffe, range_length = 5"
            ))
        );
    }

    #[test]
    fn test_initial_register() {
        let mut cpu = CPU::new();
        cpu.reset();

        assert_eq!(cpu.reg_a, 0);
        assert_eq!(cpu.reg_x, 0);
        assert_eq!(cpu.reg_y, 0);
        assert_eq!(cpu.reg_status, Status::empty());
        assert_eq!(cpu.pc, 0x00);
    }

    #[test]
    fn test_lda_immediate_load_data() {
        let mut cpu = CPU::new();
        let program = vec![0xa9, 0b0000_1111, 0x00];

        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_a, 0b0000_1111);
        assert_eq!(cpu.reg_status, Status::empty());
    }

    #[test]
    fn test_lda_immediate_negative_flag() {
        let mut cpu = CPU::new();
        let program = vec![0xa9, 0b1000_1111, 0x00];

        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_a, 0b1000_1111);
        assert_eq!(cpu.reg_status, Status::N);
    }

    #[test]
    fn test_lda_immediate_zero_flag() {
        let mut cpu = CPU::new();
        let program = vec![0xa9, 0x00, 0x00];

        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_a, 0x00);
        assert_eq!(cpu.reg_status, Status::Z);
    }

    #[test]
    fn test_tax_load_data() {
        let mut cpu = CPU::new();
        // LDA #$8f
        // TAX
        // BRK
        let program = vec![0xa9, 0b0111_1111, 0xaa, 0x00];

        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_a, 0b0111_1111);
        assert_eq!(cpu.reg_x, 0b0111_1111);
        assert_eq!(cpu.reg_status, Status::empty());
    }

    #[test]
    fn test_tax_negative_flag() {
        let mut cpu = CPU::new();
        // LDA #$ff
        // TAX
        // BRK
        let program = vec![0xa9, 0b1111_1111, 0xaa, 0x00];

        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_a, 0b1111_1111);
        assert_eq!(cpu.reg_x, 0b1111_1111);
        assert_eq!(cpu.reg_status, Status::N);
    }

    #[test]
    fn test_tax_zero_flag() {
        let mut cpu = CPU::new();
        // LDA #$ff
        // TAX
        // BRK
        let program = vec![0xa9, 0x00, 0xaa, 0x00];

        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_a, 0x00);
        assert_eq!(cpu.reg_x, 0x00);
        assert_eq!(cpu.reg_status, Status::Z);
    }

    #[test]
    fn test_inx() {
        let mut cpu = CPU::new();
        // INX
        // INX
        let program = vec![0xe8, 0xe8, 0x00];

        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_x, 0x02);
        assert_eq!(cpu.reg_status, Status::empty());
    }

    #[test]
    fn test_inx_zero_flag() {
        let mut cpu = CPU::new();
        let mut program = vec![0; 8000];
        for i in 0..0x100 {
            program[i] = 0xe8;
        }

        // INX * 256
        // BRK
        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_x, 0x00);
        assert_eq!(cpu.reg_status, Status::Z);
    }

    #[test]
    fn test_inx_negative_flag() {
        let mut cpu = CPU::new();
        let mut program = vec![0; 8000];
        for i in 0..0xf0 {
            program[i] = 0xe8;
        }
        // INX * 0xf0
        // BRK
        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_x, 0xf0);
        assert_eq!(cpu.reg_status, Status::N);
    }

    #[test]
    fn test_inx_overflow() {
        let mut cpu = CPU::new();
        let mut program = vec![0; 8000];
        for i in 0..0x101 {
            program[i] = 0xe8;
        }

        // INX * 257
        // BRK
        assert_eq!(cpu.interpret(&program), Ok(()));

        assert_eq!(cpu.reg_x, 1)
    }
}

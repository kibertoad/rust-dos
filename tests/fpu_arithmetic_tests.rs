use rust_dos::cpu::{Cpu, FpuFlags};
use rust_dos::f80::F80;

mod testrunners;

#[test]
fn test_fpu_arithmetic_matrix() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    
    // Setup: ST(1) = 100.0, ST(0) = 20.0
    let mut f100 = F80::new(); f100.set_f64(100.0);
    let mut f20 = F80::new(); f20.set_f64(20.0);

    let reset_stack = |c: &mut Cpu, val0: F80, val1: F80| {
        while c.fpu_top != 0 { c.fpu_pop(); } // Clear stack
        c.fpu_push(val1); // ST(1)
        c.fpu_push(val0); // ST(0)
    };

    // --- 1. ADDITION ---
    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xD8, 0xC1]); // FADD ST(0), ST(1) -> ST(0)=120
    assert_eq!(cpu.fpu_get(0).get_f64(), 120.0);

    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xDC, 0xC1]); // FADD ST(1), ST(0) -> ST(1)=120
    assert_eq!(cpu.fpu_get(1).get_f64(), 120.0);

    // --- 2. SUBTRACTION (The "08" Killer) ---
    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xD8, 0xE1]); // FSUB ST(0), ST(1) -> 20 - 100 = -80
    assert_eq!(cpu.fpu_get(0).get_f64(), -80.0);

    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xDC, 0xE9]); // FSUB ST(1), ST(0) -> 100 - 20 = 80
    assert_eq!(cpu.fpu_get(1).get_f64(), 80.0);

    // --- 3. REVERSE SUBTRACTION ---
    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xD8, 0xE9]); // FSUBR ST(0), ST(1) -> 100 - 20 = 80
    assert_eq!(cpu.fpu_get(0).get_f64(), 80.0);

    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xDC, 0xE1]); // FSUBR ST(1), ST(0) -> 20 - 100 = -80
    assert_eq!(cpu.fpu_get(1).get_f64(), -80.0);

    // --- 4. DIVISION ---
    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xD8, 0xF1]); // FDIV ST(0), ST(1) -> 20 / 100 = 0.2
    assert_eq!(cpu.fpu_get(0).get_f64(), 0.2);

    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xDC, 0xF9]); // FDIV ST(1), ST(0) -> 100 / 20 = 5.0
    assert_eq!(cpu.fpu_get(1).get_f64(), 5.0);

    // --- 5. REVERSE DIVISION ---
    reset_stack(&mut cpu, f20, f100);
    testrunners::run_fpu_code(&mut cpu, &[0xD8, 0xF9]); // FDIVR ST(0), ST(1) -> 100 / 20 = 5.0
    assert_eq!(cpu.fpu_get(0).get_f64(), 5.0);
}

#[test]
fn test_fadd_and_fsub_real() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    let mut f1 = F80::new(); f1.set_f64(10.5);
    let mut f2 = F80::new(); f2.set_f64(2.5);
    
    cpu.fpu_push(f1); // ST(1) = 10.5
    cpu.fpu_push(f2); // ST(0) = 2.5

    // FADD ST(1), ST(0) -> 0xDC C1
    // Dest: ST(1)
    testrunners::run_fpu_code(&mut cpu, &[0xDC, 0xC1]);
    
    // ST(0) remains 2.5
    assert_eq!(cpu.fpu_get(0).get_f64(), 2.5);
    // ST(1) becomes 13.0
    assert_eq!(cpu.fpu_get(1).get_f64(), 13.0);
}

#[test]
fn test_faddp_behavior() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    let mut f1 = F80::new(); f1.set_f64(5.0);
    let mut f2 = F80::new(); f2.set_f64(1.0);
    cpu.fpu_push(f1);
    cpu.fpu_push(f2); // ST(0)=1, ST(1)=5

    // FADDP ST(1), ST(0) -> DE C1
    testrunners::run_fpu_code(&mut cpu, &[0xDE, 0xC1]);

    // Result should be 6.0 and ST(0) should be popped
    assert_eq!(cpu.fpu_get(0).get_f64(), 6.0);
    // Check if tag of old ST(1) (now ST(0)) is valid and previous is empty
    assert_eq!(cpu.fpu_top, 7); // Pushed twice (-2), Popped once (+1) = 7 (mod 8)
}

#[test]
fn test_fiadd_integer_memory() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    let mut f0 = F80::new(); f0.set_f64(100.0);
    cpu.fpu_push(f0);

    // Setup memory: [0x2000] = 50 (Int32)
    let addr = 0x2000;
    cpu.bus.write_32(addr, 50);

    // DA 06 00 20 is FIADD dword [2000] in 16-bit mode
    testrunners::run_fpu_code(&mut cpu, &[0xDA, 0x06, 0x00, 0x20]);

    assert_eq!(cpu.fpu_get(0).get_f64(), 150.0);
}

#[test]
fn test_fsqrt_and_invalid_op() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    
    // 1. Valid Case: sqrt(16.0) = 4.0
    let mut f16 = F80::new(); 
    f16.set_f64(16.0);
    cpu.fpu_push(f16);
    
    // D9 FA: FSQRT
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xFA]); 
    
    assert_eq!(cpu.fpu_get(0).get_f64(), 4.0);
    // Ensure Invalid Operation (IE) is NOT set
    assert!(!cpu.get_fpu_flags().contains(FpuFlags::IE));

    // 2. Invalid Case: sqrt(-1.0) -> IE Exception
    let mut fneg = F80::new(); 
    fneg.set_f64(-1.0);
    cpu.fpu_push(fneg);
    
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xFA]);
    
    // Result should be Real Indefinite (NaN)
    assert!(cpu.fpu_get(0).is_nan());
    // Ensure Invalid Operation (IE) IS set
    assert!(cpu.get_fpu_flags().contains(FpuFlags::IE));
}

#[test]
fn test_fprem_partial_remainder() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    
    // 10.0 / 3.0 -> remainder 1.0 (Quotient = 3)
    let mut f3 = F80::new(); f3.set_f64(3.0);
    let mut f10 = F80::new(); f10.set_f64(10.0);
    cpu.fpu_push(f3);   // ST(1)
    cpu.fpu_push(f10);  // ST(0)

    // D9 F8: FPREM
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xF8]); 

    assert_eq!(cpu.fpu_get(0).get_f64(), 1.0);

    // Quotient Q = 3 (Binary 011)
    // Bit 2 (4) -> C0: 0
    // Bit 1 (2) -> C3: 1
    // Bit 0 (1) -> C1: 1
    let flags = cpu.get_fpu_flags();
    
    assert!(flags.contains(FpuFlags::C3), "C3 should be set (Q bit 1 is 1)");
    assert!(flags.contains(FpuFlags::C1), "C1 should be set (Q bit 0 is 1)");
    assert!(!flags.contains(FpuFlags::C0), "C0 should be clear (Q bit 2 is 0)");
}

#[test]
fn test_fabs_fchs() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    let mut f_neg = F80::new(); f_neg.set_f64(-5.5);
    cpu.fpu_push(f_neg);

    // FABS
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xE1]);
    assert_eq!(cpu.fpu_get(0).get_f64(), 5.5);
    assert_eq!(cpu.fpu_get(0).get_sign(), false);

    // FCHS (change back to negative)
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xE0]);
    assert_eq!(cpu.fpu_get(0).get_f64(), -5.5);
    assert_eq!(cpu.fpu_get(0).get_sign(), true);
}

#[test]
fn test_f2xm1_exponentiation() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    
    // Test 2^0.5 - 1
    // 0.5 is within the required range (-1.0 to 1.0) for F2XM1
    let mut f_half = F80::new(); f_half.set_f64(0.5);
    cpu.fpu_push(f_half);

    // D9 F0: F2XM1
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xF0]);

    let result = cpu.fpu_get(0).get_f64();
    let expected = 2.0f64.powf(0.5) - 1.0;
    
    // Check with small epsilon for float precision
    assert!((result - expected).abs() < 1e-12);
}

#[test]
fn test_fyl2x_logarithm() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    
    // Calculate 3 * log2(8) = 3 * 3 = 9
    let mut f_y = F80::new(); f_y.set_f64(3.0);
    let mut f_x = F80::new(); f_x.set_f64(8.0);
    
    cpu.fpu_push(f_y); // ST(1)
    cpu.fpu_push(f_x); // ST(0)

    // D9 F1: FYL2X (Result in ST(1), Pops ST(0))
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xF1]);

    assert_eq!(cpu.fpu_get(0).get_f64(), 9.0);
}

#[test]
fn test_fxtract_decomposition() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    let mut f12 = F80::new(); f12.set_f64(12.0); // 1.5 * 2^3
    cpu.fpu_push(f12);

    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xF4]);

    assert_eq!(cpu.fpu_get(0).get_f64(), 1.5);
    assert_eq!(cpu.fpu_get(1).get_f64(), 3.0);
}

#[test]
fn test_fscale_powers_of_two() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    let mut f_exp = F80::new(); f_exp.set_f64(2.0);
    let mut f_val = F80::new(); f_val.set_f64(3.0);
    
    cpu.fpu_push(f_exp);
    cpu.fpu_push(f_val);

    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xFD]);

    assert_eq!(cpu.fpu_get(0).get_f64(), 12.0);
}

#[test]
fn test_fadd_diagnostic() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    let mut f1 = F80::new(); f1.set_f64(10.5);
    let mut f2 = F80::new(); f2.set_f64(2.5);
    
    cpu.fpu_push(f1); 
    println!("After Push 1: TOP={}, Tags={:?}", cpu.fpu_top, cpu.fpu_tags);
    
    cpu.fpu_push(f2); 
    println!("After Push 2: TOP={}, Tags={:?}", cpu.fpu_top, cpu.fpu_tags);

    // FADD ST(1), ST(0)
    testrunners::run_fpu_code(&mut cpu, &[0xDC, 0xC1]);

    println!("After FADD: TOP={}, ST(0)={:?}, ST(1)={:?}", 
        cpu.fpu_top, cpu.fpu_get(0).get_f64(), cpu.fpu_get(1).get_f64());
    
    for i in 0..8 {
        println!("Phys Reg {}: {:?} (Tag: {})", i, cpu.fpu_reg(i).get_f64(), cpu.fpu_tags[i]);
    }

    assert_eq!(cpu.fpu_get(1).get_f64(), 13.0);
}

#[test]
fn test_fsub_variants() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    let mut f10 = F80::new(); f10.set_f64(10.0);
    let mut f2 = F80::new(); f2.set_f64(2.0);
    cpu.fpu_push(f10); // ST(1)
    cpu.fpu_push(f2);  // ST(0)

    // Variant 1: D8 E1 -> FSUB ST(0), ST(1) 
    // ST(0) = 2.0 - 10.0 = -8.0
    testrunners::run_fpu_code(&mut cpu, &[0xD8, 0xE1]);
    assert_eq!(cpu.fpu_get(0).get_f64(), -8.0);

    // Variant 2: DC E9 -> FSUB ST(1), ST(0)
    // Resetting for test...
    cpu.fpu_set(0, f2); 
    cpu.fpu_set(1, f10);
    // ST(1) = 10.0 - 2.0 = 8.0
    testrunners::run_fpu_code(&mut cpu, &[0xDC, 0xE9]);
    assert_eq!(cpu.fpu_get(1).get_f64(), 8.0);
}

fn push_val(cpu: &mut Cpu, val: f64) {
    let mut f = F80::new();
    f.set_f64(val);
    cpu.fpu_push(f);
}

#[test]
fn test_fsubp_st2_st0() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    // Setup: ST(0)=1.0, ST(1)=10.0, ST(2)=100.0
    push_val(&mut cpu, 100.0);
    push_val(&mut cpu, 10.0);
    push_val(&mut cpu, 1.0);

    // DE EA: FSUBP ST(2), ST(0)
    // ST(2) = ST(2) - ST(0) -> 100.0 - 1.0 = 99.0
    // Pop ST(0)
    // New Stack: ST(0)=10.0, ST(1)=99.0
    testrunners::run_fpu_code(&mut cpu, &[0xDE, 0xEA]);

    assert_eq!(cpu.fpu_get(0).get_f64(), 10.0, "ST(1) should become ST(0)");
    assert_eq!(cpu.fpu_get(1).get_f64(), 99.0, "ST(2) should have been modified and become ST(1)");
}

#[test]
fn test_trigonometry() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    
    // FSIN of PI/2
    push_val(&mut cpu, std::f64::consts::FRAC_PI_2);
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xFE]); // FSIN
    let val = cpu.fpu_get(0).get_f64();
    assert!((val - 1.0).abs() < 0.0001);

    // FPTAN of PI/4 (tan = 1.0)
    // Should result in ST(0)=1.0, ST(1)=1.0 (It pushes 1.0 after calc)
    push_val(&mut cpu, std::f64::consts::FRAC_PI_4);
    testrunners::run_fpu_code(&mut cpu, &[0xD9, 0xF2]); // FPTAN
    
    assert!((cpu.fpu_get(0).get_f64() - 1.0).abs() < 0.0001); // The pushed 1.0
    assert!((cpu.fpu_get(1).get_f64() - 1.0).abs() < 0.0001); // The result tan(pi/4)
}

/// An infinity has the integer bit set in 80 bits (Intel SDM vol. 1, "Real
/// Number and NaN Encodings"): one loaded as 80 bits is an infinity to
/// FMUL and FXAM, and one a division by 0 made stores as 80 bits with it.
#[test]
fn test_80_bit_infinities_have_the_integer_bit() {
    let mut cpu = Cpu::new(std::path::PathBuf::from("."));
    cpu.bus.write_64(0x1000, 0x8000_0000_0000_0000);
    cpu.bus.write_16(0x1008, 0x7FFF);
    // DB 2E 00 10: FLD TBYTE PTR [1000]; D9 E5: FXAM
    testrunners::run_cpu_code(&mut cpu, &[0xDB, 0x2E, 0x00, 0x10]);
    testrunners::run_cpu_code(&mut cpu, &[0xD9, 0xE5]);
    let cc = cpu.get_fpu_flags() & (FpuFlags::C3 | FpuFlags::C2 | FpuFlags::C0);
    assert_eq!(cc, FpuFlags::C2 | FpuFlags::C0, "FXAM: infinity");
    // D9 E8: FLD1; DE C9: FMULP; DD 1E 10 10: FSTP QWORD PTR [1010]
    testrunners::run_cpu_code(&mut cpu, &[0xD9, 0xE8]);
    testrunners::run_cpu_code(&mut cpu, &[0xDE, 0xC9]);
    testrunners::run_cpu_code(&mut cpu, &[0xDD, 0x1E, 0x10, 0x10]);
    assert_eq!(cpu.bus.read_64(0x1010), f64::INFINITY.to_bits(), "inf * 1");

    // D9 E8: FLD1; D9 EE: FLDZ; DE F9: FDIVP (1/0); DB 3E 20 10: FSTP TBYTE PTR [1020]
    for code in [&[0xD9, 0xE8][..], &[0xD9, 0xEE], &[0xDE, 0xF9], &[0xDB, 0x3E, 0x20, 0x10]] {
        testrunners::run_cpu_code(&mut cpu, code);
    }
    assert_eq!((cpu.bus.read_16(0x1028), cpu.bus.read_64(0x1020)), (0x7FFF, 0x8000_0000_0000_0000), "1/0");
}

/// FADD and FSUB of 80-bit infinities, NaNs and sums past the largest
/// number, as an x87 with its exceptions masked does them: the expected
/// results and exception bits (IE 01h, OE and PE 28h) are what FLD TBYTE,
/// FLD TBYTE, FADDP or FSUBRP, FSTP TBYTE gave on an x87.
#[test]
fn test_fadd_fsub_of_infinities_nans_and_overflow() {
    let inf = (0x7FFF, 0x8000_0000_0000_0000);
    let ninf = (0xFFFF, 0x8000_0000_0000_0000);
    let one = (0x3FFF, 0x8000_0000_0000_0000);
    let qnan = (0x7FFF, 0xC000_0000_0000_1234);
    let qnan2 = (0xFFFF, 0xC000_0000_0000_5678);
    let snan = (0x7FFF, 0x8000_0000_0000_1234);
    let big = (0x7FFE, 0xFFFF_FFFF_FFFF_FFFF);
    let nbig = (0xFFFE, 0xFFFF_FFFF_FFFF_FFFF);
    let indefinite = (0xFFFF, 0xC000_0000_0000_0000);
    // (a, b, a - b rather than a + b, result, exceptions)
    let cases: [((u16, u64), (u16, u64), bool, (u16, u64), u16); 14] = [
        (inf, inf, false, inf, 0),
        (inf, inf, true, indefinite, 0x01),
        (inf, ninf, false, indefinite, 0x01),
        (one, inf, true, ninf, 0),
        (inf, one, true, inf, 0),
        (ninf, one, false, ninf, 0),
        (qnan, one, false, qnan, 0),
        (one, qnan, true, qnan, 0),
        (one, snan, false, (0x7FFF, 0xC000_0000_0000_1234), 0x01),
        (qnan, qnan2, false, qnan2, 0),
        (qnan2, qnan, true, qnan2, 0),
        (qnan, inf, true, qnan, 0),
        (big, big, false, inf, 0x28),
        (nbig, big, true, ninf, 0x28),
    ];
    for (a, b, sub, want, flags) in cases {
        let mut cpu = Cpu::new(std::path::PathBuf::from("."));
        for (at, (se, man)) in [(0x1000, a), (0x1010, b)] {
            cpu.bus.write_64(at, man);
            cpu.bus.write_16(at + 8, se);
        }
        // DB 2E 10 10: FLD TBYTE PTR [1010]; DB 2E 00 10: FLD TBYTE PTR [1000]
        testrunners::run_cpu_code(&mut cpu, &[0xDB, 0x2E, 0x10, 0x10]);
        testrunners::run_cpu_code(&mut cpu, &[0xDB, 0x2E, 0x00, 0x10]);
        // DE E1: FSUBRP ST(1), ST(0) (ST(1) = ST(0) - ST(1) = a - b); DE C1: FADDP
        testrunners::run_cpu_code(&mut cpu, if sub { &[0xDE, 0xE1] } else { &[0xDE, 0xC1] });
        // DB 3E 20 10: FSTP TBYTE PTR [1020]
        testrunners::run_cpu_code(&mut cpu, &[0xDB, 0x3E, 0x20, 0x10]);
        let got = (cpu.bus.read_16(0x1028), cpu.bus.read_64(0x1020));
        let case = format!("{:04X}:{:016X} {} {:04X}:{:016X}", a.0, a.1, if sub { '-' } else { '+' }, b.0, b.1);
        assert_eq!(got, want, "{}", case);
        let exceptions = cpu.get_fpu_flags().bits() & 0x3F;
        assert_eq!(exceptions, flags, "exceptions of {}", case);
    }
}

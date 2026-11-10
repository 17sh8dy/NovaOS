use std::fmt::Write;

/// Vectors for which the CPU pushes an error code before the return frame.
const WITH_ERROR_CODE: [u32; 10] = [8, 10, 11, 12, 13, 14, 17, 21, 29, 30];

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg-bins=-T{dir}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=build.rs");

    // One tiny entry stub per interrupt vector. Where the CPU pushes no error
    // code, a dummy 0 is pushed so every stub leaves the same stack layout.
    let mut asm = String::new();
    asm.push_str(".section .text.nova_isr,\"ax\"\n");
    for v in 0..256u32 {
        writeln!(asm, "nova_isr_{v}:").unwrap();
        if !WITH_ERROR_CODE.contains(&v) {
            asm.push_str("    push 0\n");
        }
        writeln!(asm, "    push {v}\n    jmp nova_isr_common").unwrap();
    }
    asm.push_str(".section .rodata.nova_isr_table,\"a\"\n.balign 8\n.global nova_isr_table\nnova_isr_table:\n");
    for v in 0..256u32 {
        writeln!(asm, "    .quad nova_isr_{v}").unwrap();
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("isr_stubs.s");
    std::fs::write(out, asm).unwrap();
}

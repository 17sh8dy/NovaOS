//! SBAT ("Secure Boot Advanced Targeting") metadata.
//!
//! With Secure Boot on, Nova boots through *shim* (a small loader signed by
//! Microsoft that Linux distributions ship). Current shims refuse to start a
//! second-stage loader that has no `.sbat` section. The section is CSV text:
//! the SBAT format version, then one line per component with a *generation*
//! number. If a security bug is ever found in NovaBoot, the generation is
//! raised, and revocation lists can then reject the old, vulnerable builds.
//!
//! Format: <https://github.com/rhboot/shim/blob/main/SBAT.md>.

/// Keep the generation (the `1` after `novaboot`) in step with security fixes.
const SBAT: &[u8] = b"sbat,1,SBAT Version,sbat,1,https://github.com/rhboot/shim/blob/main/SBAT.md\n\
novaboot,1,NovaOS,novaboot,0.1.0,https://github.com/17sh8dy/NovaOS\n";

#[used]
#[unsafe(link_section = ".sbat")]
static SBAT_SECTION: [u8; SBAT.len()] = {
    let mut out = [0u8; SBAT.len()];
    let mut i = 0;
    while i < SBAT.len() {
        out[i] = SBAT[i];
        i += 1;
    }
    out
};

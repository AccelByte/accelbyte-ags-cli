//! Minimal ELF header inspection for the AMS entrypoint accept matrix.
//!
//! AMS only runs 64-bit little-endian x86-64 or aarch64 dedicated servers, so
//! the whole decision is made from `e_ident` plus `e_machine`. Anything that is
//! not an ELF file at all (a PE binary, a text file, a truncated stub) is
//! reported as `None` rather than an error, mirroring armada-cli: the caller
//! turns that into "must be an ELF binary or a shell script".

use super::TargetArchitecture;

/// Bytes of the ELF identification block plus `e_type`/`e_machine`.
const HEADER_PREFIX_LEN: usize = 20;

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;

/// Classify `header` — the leading bytes of a candidate executable — into the
/// AMS target architecture it declares.
///
/// Returns `None` for any file that is not an accepted ELF binary, including
/// valid ELF files for a class, byte order, or machine AMS cannot run.
pub(crate) fn detect_architecture(header: &[u8]) -> Option<TargetArchitecture> {
    if header.len() < HEADER_PREFIX_LEN {
        return None;
    }
    if header[0..4] != ELF_MAGIC {
        return None;
    }
    if header[4] != ELFCLASS64 || header[5] != ELFDATA2LSB || header[6] != EV_CURRENT {
        return None;
    }
    match u16::from_le_bytes([header[18], header[19]]) {
        EM_X86_64 => Some(TargetArchitecture::LinuxX86_64),
        EM_AARCH64 => Some(TargetArchitecture::LinuxArm64),
        _ => None,
    }
}

/// How many leading bytes [`detect_architecture`] needs from a file.
pub(crate) const fn header_prefix_len() -> usize {
    HEADER_PREFIX_LEN
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 20-byte ELF header prefix with the given class, data encoding,
    /// and machine, so each axis of the accept matrix can be varied alone.
    fn elf_header(class: u8, data: u8, machine: u16) -> Vec<u8> {
        let mut header = vec![0u8; HEADER_PREFIX_LEN];
        header[0..4].copy_from_slice(&ELF_MAGIC);
        header[4] = class;
        header[5] = data;
        header[6] = EV_CURRENT;
        header[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        header[18..20].copy_from_slice(&machine.to_le_bytes());
        header
    }

    #[test]
    fn test_detects_x86_64() {
        assert_eq!(
            detect_architecture(&elf_header(ELFCLASS64, ELFDATA2LSB, EM_X86_64)),
            Some(TargetArchitecture::LinuxX86_64)
        );
    }

    #[test]
    fn test_detects_aarch64() {
        assert_eq!(
            detect_architecture(&elf_header(ELFCLASS64, ELFDATA2LSB, EM_AARCH64)),
            Some(TargetArchitecture::LinuxArm64)
        );
    }

    #[test]
    fn test_rejects_32_bit() {
        assert_eq!(
            detect_architecture(&elf_header(1, ELFDATA2LSB, EM_X86_64)),
            None
        );
    }

    #[test]
    fn test_rejects_big_endian() {
        assert_eq!(
            detect_architecture(&elf_header(ELFCLASS64, 2, EM_AARCH64)),
            None
        );
    }

    #[test]
    fn test_rejects_unsupported_machine() {
        // EM_RISCV — a real ELF, but not one AMS can run.
        assert_eq!(
            detect_architecture(&elf_header(ELFCLASS64, ELFDATA2LSB, 243)),
            None
        );
    }

    #[test]
    fn test_rejects_pe_binary() {
        // `MZ` DOS stub — what a Windows executable starts with.
        let mut header = vec![0u8; HEADER_PREFIX_LEN];
        header[0] = b'M';
        header[1] = b'Z';
        assert_eq!(detect_architecture(&header), None);
    }

    #[test]
    fn test_rejects_truncated_file() {
        assert_eq!(detect_architecture(&ELF_MAGIC), None);
        assert_eq!(detect_architecture(&[]), None);
    }
}

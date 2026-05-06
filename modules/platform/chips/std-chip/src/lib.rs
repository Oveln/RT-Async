use std::process::exit;

pub struct StdChip;

impl platform_traits::Chip for StdChip {
    fn shutdown() -> ! {
        exit(0)
    }

    fn put_str(s: &str) {
        print!("{}",s);
    }

    unsafe fn pend() {
    }
}
use core::fmt::Write;
use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};

struct ConsoleWriter;

impl Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // boot 早期（derive_console 之前）console 可能尚未就绪：此时静默丢弃，
        // 不 panic——否则 boot 期间任何 log（如 PLIC probe）会触发 console()
        // 的 expect 而死循环。运行期 console 必已就绪，正常输出。
        if let Some(con) = crate::driver::try_console() {
            con.write(s.as_bytes());
        }
        Ok(())
    }
}

pub struct Logger;

impl Logger {
    pub const fn new() -> Self {
        Self
    }

    pub fn init(&'static self, max_level: LevelFilter) -> Result<(), SetLoggerError> {
        log::set_logger(self)?;
        log::set_max_level(max_level);
        Ok(())
    }
}

impl Log for Logger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut w = ConsoleWriter;
        let _ = writeln!(w, "[{}] {}", record.level(), record.args());
    }

    fn flush(&self) {}
}

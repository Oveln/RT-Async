use core::fmt::Write;
use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};

use crate::{Chip, ChipImpl};

struct ChipWriter(core::marker::PhantomData<fn() -> ChipImpl>);

impl Write for ChipWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        ChipImpl::put_str(s);
        Ok(())
    }
}

pub struct Logger {
    _marker: core::marker::PhantomData<fn() -> ChipImpl>,
}

impl Logger {
    pub const fn new() -> Self {
        Self {
            _marker: core::marker::PhantomData,
        }
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
        let mut w = ChipWriter(core::marker::PhantomData);
        let _ = writeln!(w, "[{}] {}", record.level(), record.args());
    }

    fn flush(&self) {}
}

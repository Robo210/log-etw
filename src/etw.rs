use std::{cell::RefCell, time::SystemTime, pin::Pin};
use crate::logger::{map_level, ProviderWrapper};
use chrono::{Datelike, Timelike};
use tracelogging_dynamic::EventBuilder;
use tracelogging::*;

thread_local! {static EBW: std::cell::RefCell<EventBuilder>  = RefCell::new(EventBuilder::new());}

struct Win32SystemTime {
    st: [u16; 8],
}

impl From<std::time::SystemTime> for Win32SystemTime {
    fn from(value: std::time::SystemTime) -> Self {
        let dt = chrono::DateTime::from(value);

        Win32SystemTime {
            st: [
                dt.year() as u16,
                dt.month() as u16,
                0,
                dt.day() as u16,
                dt.hour() as u16,
                dt.minute() as u16,
                dt.second() as u16,
                (dt.nanosecond() / 1000000) as u16,
            ],
        }
    }
}

impl ProviderWrapper {
    pub(crate) fn write_record(self: Pin<&Self>, timestamp: SystemTime, record: &log::Record) {
        let event_name = "Event"; // TODO

        let level = map_level(record.level());
        let keyword = 0u64; // TODO

        if !self.enabled(level, keyword) {
            return;
        }

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(&event_name, level.into(), keyword, 0);
            eb.opcode(Opcode::Info);

            eb.add_systemtime("time", &Into::<Win32SystemTime>::into(timestamp).st, OutType::DateTimeUtc, 0);

            let payload = format!("{}", record.args());
            eb.add_str8("Payload", payload, OutType::Utf8, 0);

            if let Some(module_path) = record.module_path() {
                eb.add_str8("Module Path", module_path, OutType::Utf8, 0);
            }

            if let Some(file) = record.file() {
                eb.add_str8("File", file, OutType::Utf8, 0);

                if let Some(line) = record.line() {
                    eb.add_u32("Line", line, OutType::Unsigned, 0);
                }
            }

            let _ = eb.write(&self.get_provider(), None, None);

            // if self.provider.enabled(Level::Informational, log_keywords)
            //     && self.exporter_config.get_export_common_schema_events()
            // {
            //     let err2 = ebw.write_common_schema_log_event(
            //         &self.provider.as_ref(),
            //         event_name,
            //         Level::Informational,
            //         log_keywords,
            //         log_record,
            //         export_payload_as_json,
            //         attributes.clone(),
            //     );

            //     err = err.and(err2);
            // }
        })
    }
}

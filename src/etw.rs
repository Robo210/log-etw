use crate::logger::{map_level, ProviderWrapper, ExporterConfig};
use chrono::{Datelike, Timelike};
use std::{cell::RefCell, pin::Pin, time::SystemTime};
use tracelogging::*;
use tracelogging_dynamic::EventBuilder;

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
    pub(crate) fn write_record(self: Pin<&Self>, timestamp: SystemTime, record: &log::Record, exporter_config: &ExporterConfig) {
        let event_name = "Event"; // TODO

        let level = map_level(record.level());
        let keyword = 0u64; // TODO

        if !self.enabled(level, keyword) {
            return;
        }

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            if !exporter_config.common_schema {
                eb.reset(&event_name, level.into(), keyword, 0);
                eb.opcode(Opcode::Info);

                eb.add_systemtime(
                    "time",
                    &Into::<Win32SystemTime>::into(timestamp).st,
                    OutType::DateTimeUtc,
                    0,
                );

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
            } else {
                eb.reset(&event_name, level.into(), keyword, 0);
                eb.opcode(Opcode::Info);

                let mut parta_field_count = 1;
                let mut span_id: Option<[u8; 16]> = None;
                let mut trace_id: Option<[u8; 32]> = None;
                #[cfg(any(feature="spans"))]
                {
                    use std::io::Write;

                    let active_span_id: [u8; 16];
                    let active_trace_id: [u8; 32];

                    (active_span_id, active_trace_id) = opentelemetry_api::trace::get_active_span(|span| {
                        if span.span_context().span_id() != opentelemetry_api::trace::SpanId::INVALID {
                            let trace_id = unsafe {
                                let mut trace_id = std::mem::MaybeUninit::<[u8; 32]>::uninit();
                                let mut cur = std::io::Cursor::new((&mut *trace_id.as_mut_ptr()).as_mut_slice());
                                write!(&mut cur, "{:32x}", span.span_context().trace_id()).expect("!write");
                                trace_id.assume_init()
                            };
                    
                            let span_id = unsafe {
                                let mut span_id = std::mem::MaybeUninit::<[u8; 16]>::uninit();
                                let mut cur = std::io::Cursor::new((&mut *span_id.as_mut_ptr()).as_mut_slice());
                                write!(&mut cur, "{:16x}", span.span_context().span_id()).expect("!write");
                                span_id.assume_init()
                            };

                            parta_field_count += 1;
                            (span_id, trace_id)
                        } else {
                            ([0; 16], [0; 32])
                        }
                    });

                    span_id = Some(active_span_id);
                    trace_id = Some(active_trace_id);
                }

                eb.add_u16("__csver__", 0x0401, OutType::Signed, 0);
                eb.add_struct("PartA", 2 /* + exts.len() as u8*/, 0);
                {
                    let time: String = chrono::DateTime::to_rfc3339(
                        &chrono::DateTime::<chrono::Utc>::from(timestamp),
                    );
                    eb.add_str8("time", time, OutType::Utf8, 0);

                    if trace_id.is_some() {
                        eb.add_struct("ext_dt", 2, 0);
                        {
                            eb.add_str8("traceId", &trace_id.unwrap(), OutType::Utf8, 0); // TODO
                            eb.add_str8("spanId", &span_id.unwrap(), OutType::Utf8, 0); // TODO
                        }
                    }
                }

                eb.add_struct("PartB", 5, 0);
                {
                    eb.add_str8("_typeName", "Log", OutType::Utf8, 0);
                    eb.add_str8("name", event_name, OutType::Utf8, 0);

                    eb.add_str8(
                        "eventTime",
                        &chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                            timestamp,
                        )),
                        OutType::Utf8,
                        0,
                    );

                    eb.add_u8("severityNumber", record.level() as u8, OutType::Unsigned, 0);
                    eb.add_str8("severityText", record.level().as_str(), OutType::Utf8, 0);
                }

                eb.add_struct("PartC", 1, 0);
                {
                    let payload = format!("{}", record.args());
                    eb.add_str8("Payload", payload, OutType::Utf8, 0);
                }

                let _ = eb.write(&self.get_provider(), None, None);
            }
        })
    }
}

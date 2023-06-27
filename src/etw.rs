use crate::logger::{map_level, ExporterConfig, ProviderWrapper};
use chrono::{Datelike, Timelike};
#[cfg(any(feature = "kv_unstable", feature = "kv_unstable_json"))]
use log::kv::{source, value::Visit, Visitor};
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
    pub(crate) fn write_record(
        self: Pin<&Self>,
        timestamp: SystemTime,
        event_name: &str,
        keyword: u64,
        record: &log::Record,
        exporter_config: &ExporterConfig,
    ) {
        let level = map_level(record.level());

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

                #[cfg(any(feature = "kv_unstable", feature = "kv_unstable_json"))]
                {
                    if cfg!(feature = "kv_unstable_json") && exporter_config.json {
                        if let Ok(json) =
                            serde_json::to_string(&source::as_map(record.key_values()))
                        {
                            eb.add_str8("Keys / Values", json, OutType::Json, 0);
                        }
                    } else {
                        struct ValueVisitor<'v, 'a> {
                            key_name: &'v str,
                            eb: &'a mut EventBuilder,
                        }
                        impl<'v, 'a> Visit<'v> for ValueVisitor<'v, 'a> {
                            fn visit_any(
                                &mut self,
                                value: log::kv::Value,
                            ) -> Result<(), log::kv::Error> {
                                self.eb.add_str8(
                                    self.key_name,
                                    value.to_string(),
                                    OutType::String,
                                    0,
                                );
                                Ok(())
                            }

                            fn visit_bool(&mut self, value: bool) -> Result<(), log::kv::Error> {
                                self.eb.add_bool32(
                                    self.key_name,
                                    value as i32,
                                    OutType::Boolean,
                                    0,
                                );
                                Ok(())
                            }

                            fn visit_borrowed_str(
                                &mut self,
                                value: &'v str,
                            ) -> Result<(), log::kv::Error> {
                                self.eb.add_str8(self.key_name, value, OutType::String, 0);
                                Ok(())
                            }

                            fn visit_str(&mut self, value: &str) -> Result<(), log::kv::Error> {
                                self.eb.add_str8(self.key_name, value, OutType::String, 0);
                                Ok(())
                            }

                            fn visit_char(&mut self, value: char) -> Result<(), log::kv::Error> {
                                self.eb
                                    .add_u8(self.key_name, value as u8, OutType::String, 0);
                                Ok(())
                            }

                            fn visit_f64(&mut self, value: f64) -> Result<(), log::kv::Error> {
                                self.eb.add_f64(self.key_name, value, OutType::Signed, 0);
                                Ok(())
                            }

                            fn visit_i128(&mut self, value: i128) -> Result<(), log::kv::Error> {
                                unsafe {
                                    self.eb.add_u64_sequence(
                                        self.key_name,
                                        core::slice::from_raw_parts(
                                            &value.to_le_bytes() as *const u8 as *const u64,
                                            2,
                                        ),
                                        OutType::Hex,
                                        0,
                                    );
                                }
                                Ok(())
                            }

                            fn visit_u128(&mut self, value: u128) -> Result<(), log::kv::Error> {
                                unsafe {
                                    self.eb.add_u64_sequence(
                                        self.key_name,
                                        core::slice::from_raw_parts(
                                            &value.to_le_bytes() as *const u8 as *const u64,
                                            2,
                                        ),
                                        OutType::Hex,
                                        0,
                                    );
                                }
                                Ok(())
                            }

                            fn visit_u64(&mut self, value: u64) -> Result<(), log::kv::Error> {
                                self.eb.add_u64(self.key_name, value, OutType::Unsigned, 0);
                                Ok(())
                            }

                            fn visit_i64(&mut self, value: i64) -> Result<(), log::kv::Error> {
                                self.eb.add_i64(self.key_name, value, OutType::Signed, 0);
                                Ok(())
                            }
                        }

                        struct KvVisitor<'a> {
                            eb: &'a mut EventBuilder,
                        }
                        impl<'kvs> Visitor<'kvs> for KvVisitor<'_> {
                            fn visit_pair(
                                &mut self,
                                key: log::kv::Key<'kvs>,
                                value: log::kv::Value<'kvs>,
                            ) -> Result<(), log::kv::Error> {
                                let mut value_visitor = ValueVisitor {
                                    key_name: key.as_str(),
                                    eb: &mut self.eb,
                                };
                                let _ = value.visit(&mut value_visitor);

                                Ok(())
                            }
                        }

                        let _ = record.key_values().visit(&mut KvVisitor { eb: &mut eb });
                    }
                }

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

                let parta_field_count;
                let span_id: Option<[u8; 16]>;
                let trace_id: Option<[u8; 32]>;
                #[cfg(any(feature = "spans"))]
                {
                    use std::io::Write;

                    let active_span_id: [u8; 16];
                    let active_trace_id: [u8; 32];

                    (active_span_id, active_trace_id) =
                        opentelemetry_api::trace::get_active_span(|span| {
                            if span.span_context().span_id()
                                != opentelemetry_api::trace::SpanId::INVALID
                            {
                                let trace_id = unsafe {
                                    let mut trace_id = std::mem::MaybeUninit::<[u8; 32]>::uninit();
                                    let mut cur = std::io::Cursor::new(
                                        (&mut *trace_id.as_mut_ptr()).as_mut_slice(),
                                    );
                                    write!(&mut cur, "{:32x}", span.span_context().trace_id())
                                        .expect("!write");
                                    trace_id.assume_init()
                                };

                                let span_id = unsafe {
                                    let mut span_id = std::mem::MaybeUninit::<[u8; 16]>::uninit();
                                    let mut cur = std::io::Cursor::new(
                                        (&mut *span_id.as_mut_ptr()).as_mut_slice(),
                                    );
                                    write!(&mut cur, "{:16x}", span.span_context().span_id())
                                        .expect("!write");
                                    span_id.assume_init()
                                };

                                (span_id, trace_id)
                            } else {
                                ([0; 16], [0; 32])
                            }
                        });

                    parta_field_count = 2;
                    span_id = Some(active_span_id);
                    trace_id = Some(active_trace_id);
                }
                #[cfg(not(any(feature = "spans")))]
                {
                    parta_field_count = 1;
                    span_id = None;
                    trace_id = None;
                }

                eb.add_u16("__csver__", 0x0401, OutType::Signed, 0);
                eb.add_struct("PartA", parta_field_count, 0);
                {
                    let time: String = chrono::DateTime::to_rfc3339(
                        &chrono::DateTime::<chrono::Utc>::from(timestamp),
                    );
                    eb.add_str8("time", time, OutType::Utf8, 0);

                    if trace_id.is_some() {
                        eb.add_struct("ext_dt", 2, 0);
                        {
                            eb.add_str8("traceId", &trace_id.unwrap(), OutType::Utf8, 0);
                            eb.add_str8("spanId", &span_id.unwrap(), OutType::Utf8, 0);
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

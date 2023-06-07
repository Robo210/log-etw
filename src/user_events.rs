use crate::logger::{map_level, ExporterConfig, ProviderWrapper};
#[cfg(any(feature = "kv_unstable", feature = "kv_unstable_json"))]
use log::kv::{Visitor, source, value::Visit};
use std::{cell::RefCell, pin::Pin, time::SystemTime, sync::Arc};
use eventheader::*;
use eventheader_dynamic::EventBuilder;

thread_local! {static EBW: std::cell::RefCell<EventBuilder>  = RefCell::new(EventBuilder::new());}

impl ProviderWrapper {
    fn find_set(self: Pin<&Self>, level: eventheader_dynamic::Level, keyword: u64) -> Option<Arc<eventheader_dynamic::EventSet>> {
        self.get_provider().read().unwrap().find_set(level, keyword)
    }

    fn register_set(self: Pin<&Self>, level: eventheader_dynamic::Level, keyword: u64) -> Arc<eventheader_dynamic::EventSet> {
        self.get_provider().write().unwrap().register_set(level, keyword)
    }

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

        let es = if let Some(es) = self.find_set(level.into(), keyword) {
            es
        } else {
            self.register_set(level.into(), keyword)
        };

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            if !exporter_config.common_schema {
                eb.reset(&event_name, 0);
                eb.opcode(Opcode::Info);

                eb.add_value(
                    "time",
                    timestamp
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    FieldFormat::Time,
                    0,
                );

                let payload = format!("{}", record.args());
                eb.add_str("Payload", payload, FieldFormat::Default, 0);

                #[cfg(any(feature = "kv_unstable", feature = "kv_unstable_json"))]
                {
                    if cfg!(feature = "kv_unstable_json") && exporter_config.json {
                        if let Ok(json) = serde_json::to_string(&source::as_map(record.key_values())) {
                            eb.add_str("Keys / Values", json, FieldFormat::Default, 0);
                        }
                    } else {
                        #[allow(non_camel_case_types)]
                        enum ValueTypes {
                            None,
                            v_u64(u64),
                            v_i64(i64),
                            v_u128(u128),
                            v_i128(i128),
                            v_f64(f64),
                            v_bool(bool),
                            v_str(String), // Would be nice if we didn't have to do a heap allocation
                            v_char(char),
                        }
                        struct ValueVisitor {
                            value: ValueTypes,
                        }
                        impl<'v> Visit<'v> for ValueVisitor {
                            fn visit_any(
                                &mut self,
                                value: log::kv::Value,
                            ) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_str(value.to_string());
                                Ok(())
                            }

                            fn visit_bool(&mut self, value: bool) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_bool(value);
                                Ok(())
                            }

                            fn visit_borrowed_str(
                                &mut self,
                                value: &'v str,
                            ) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_str(value.to_string());
                                Ok(())
                            }

                            fn visit_str(&mut self, value: &str) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_str(value.to_string());
                                Ok(())
                            }

                            fn visit_char(&mut self, value: char) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_char(value);
                                Ok(())
                            }

                            fn visit_f64(&mut self, value: f64) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_f64(value);
                                Ok(())
                            }

                            fn visit_i128(&mut self, value: i128) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_i128(value);
                                Ok(())
                            }

                            fn visit_u128(&mut self, value: u128) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_u128(value);
                                Ok(())
                            }

                            fn visit_u64(&mut self, value: u64) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_u64(value);
                                Ok(())
                            }

                            fn visit_i64(&mut self, value: i64) -> Result<(), log::kv::Error> {
                                self.value = ValueTypes::v_i64(value);
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
                                    value: ValueTypes::None,
                                };
                                let _ = value.visit(&mut value_visitor);

                                unsafe {
                                    match value_visitor.value {
                                        ValueTypes::None => &mut self.eb,
                                        ValueTypes::v_bool(value) => self.eb.add_value(
                                            key.as_str(),
                                            value as i32,
                                            FieldFormat::Boolean,
                                            0,
                                        ),
                                        ValueTypes::v_u64(value) => {
                                            self.eb.add_value(key.as_str(), value, FieldFormat::Default, 0)
                                        }
                                        ValueTypes::v_i64(value) => {
                                            self.eb.add_value(key.as_str(), value, FieldFormat::SignedInt, 0)
                                        }
                                        ValueTypes::v_u128(value) => self.eb.add_value_sequence(
                                            key.as_str(),
                                            core::slice::from_raw_parts(
                                                &value.to_le_bytes() as *const u8 as *const u64,
                                                2,
                                            ),
                                            FieldFormat::HexInt,
                                            0,
                                        ),
                                        ValueTypes::v_i128(value) => self.eb.add_value_sequence(
                                            key.as_str(),
                                            core::slice::from_raw_parts(
                                                &value.to_le_bytes() as *const u8 as *const u64,
                                                2,
                                            ),
                                            FieldFormat::HexInt,
                                            0,
                                        ),
                                        ValueTypes::v_f64(value) => {
                                            self.eb.add_value(key.as_str(), value, FieldFormat::Float, 0)
                                        }
                                        ValueTypes::v_char(value) => self.eb.add_value(
                                            key.as_str(),
                                            value as u8,
                                            FieldFormat::String8,
                                            0,
                                        ),
                                        ValueTypes::v_str(value) => {
                                            self.eb.add_str(key.as_str(), value, FieldFormat::Default, 0)
                                        }
                                    };
                                }

                                Ok(())
                            }
                        }

                        let _ = record.key_values().visit(&mut KvVisitor { eb: &mut eb });
                    }
                }

                if let Some(module_path) = record.module_path() {
                    eb.add_str("Module Path", module_path, FieldFormat::Default, 0);
                }

                if let Some(file) = record.file() {
                    eb.add_str("File", file, FieldFormat::Default, 0);

                    if let Some(line) = record.line() {
                        eb.add_value("Line", line, FieldFormat::Default, 0);
                    }
                }

                let _ = eb.write(&es, None, None);
            } else {
                eb.reset(&event_name, 0);
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

                eb.add_value("__csver__", 0x0401, FieldFormat::SignedInt, 0);
                eb.add_struct("PartA", parta_field_count, 0);
                {
                    let time: String = chrono::DateTime::to_rfc3339(
                        &chrono::DateTime::<chrono::Utc>::from(timestamp),
                    );
                    eb.add_str("time", time, FieldFormat::Default, 0);

                    if trace_id.is_some() {
                        eb.add_struct("ext_dt", 2, 0);
                        {
                            eb.add_str("traceId", &trace_id.unwrap(), FieldFormat::Default, 0);
                            eb.add_str("spanId", &span_id.unwrap(), FieldFormat::Default, 0);
                        }
                    }
                }

                eb.add_struct("PartB", 5, 0);
                {
                    eb.add_str("_typeName", "Log", FieldFormat::Default, 0);
                    eb.add_str("name", event_name, FieldFormat::Default, 0);

                    eb.add_str(
                        "eventTime",
                        &chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                            timestamp,
                        )),
                        FieldFormat::Default,
                        0,
                    );

                    eb.add_value("severityNumber", record.level() as u8, FieldFormat::Default, 0);
                    eb.add_str("severityText", record.level().as_str(), FieldFormat::Default, 0);
                }

                eb.add_struct("PartC", 1, 0);
                {
                    let payload = format!("{}", record.args());
                    eb.add_str("Payload", payload, FieldFormat::Default, 0);
                }

                let _ = eb.write(&es, None, None);
            }
        })
    }
}

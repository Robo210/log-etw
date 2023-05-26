use log::Log;
use crossbeam_utils::sync::ShardedLock;
use tracelogging::Guid;
use std::collections::HashMap;
use std::borrow::Cow;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;
#[cfg(any(target_os = "windows"))]
use crate::etw::*;

// Providers go in, but never come out
lazy_static! {
    static ref PROVIDER_CACHE: ShardedLock<HashMap<String, Pin<Arc<ProviderWrapper>>>> = ShardedLock::new(HashMap::new());
}

pub(crate) struct ProviderWrapper {
    #[cfg(any(target_os = "windows"))]
    provider: tracelogging_dynamic::Provider,
    #[cfg(any(target_os = "linux"))]
    provider: eventheader_dynamic::Provider,
}

impl ProviderWrapper {
    pub(crate) fn enabled(&self, level: u8, keyword: u64) -> bool {
        #[cfg(any(target_os = "windows"))]
        return self.provider.enabled(level.into(), keyword);

        #[cfg(any(target_os = "linux"))]
        {
            let es = self.provider.find_set(level.into(), keyword);
            if es.is_some() {
                es.unwrap().enabled()
            } else {
                false
            }
        }
    }

    #[cfg(any(target_os = "windows"))]
    pub(crate) fn get_provider(self: Pin<&Self>) -> Pin<&tracelogging_dynamic::Provider> {
        unsafe { self.map_unchecked(|s| &s.provider) }
    }

    #[cfg(all(target_os = "windows"))]
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        //exporter_config: ExporterConfig<C>,
    ) -> Pin<Arc<Self>> {
        let mut options = tracelogging_dynamic::Provider::options();
        if let ProviderGroup::Windows(guid) = provider_group {
            options = *options.group_id(&guid);
        }

        let wrapper = Arc::pin(ProviderWrapper { provider: tracelogging_dynamic::Provider::new(provider_name, &options) });
        unsafe {
            wrapper.as_ref().get_provider().register();
        }

        wrapper
    }

    #[cfg(all(target_os = "linux"))]
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        _use_byte_for_bools: bool,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        let mut options = eventheader_dynamic::Provider::new_options();
        if let ProviderGroup::Linux(ref name) = provider_group {
            options = *options.group_name(&name);
        }
        let mut provider = eventheader_dynamic::Provider::new(provider_name, &options);
        user_events::register_eventsets(&mut provider, &exporter_config);

        BatchExporter {
            ebw: user_events::UserEventsExporter::new(Arc::new(provider), exporter_config),
        }
    }
}

pub(crate) enum ProviderGroup {
    Unset,
    #[allow(dead_code)]
    Windows(Guid),
    #[allow(dead_code)]
    Linux(Cow<'static, str>),
}

pub struct ExporterBuilder {
    pub(crate) provider_name: String,
    pub(crate) provider_id: Guid,
    pub(crate) provider_group: ProviderGroup,
    pub(crate) json: bool,
    pub(crate) emit_common_schema_events: bool,
    //pub(crate) exporter_config: Option<Box<dyn KeywordLevelProvider>>,
}

/// Create an exporter builder. After configuring the builder,
/// call [`ExporterBuilder::install`] to set it as the
/// [global tracer provider](https://docs.rs/opentelemetry_api/latest/opentelemetry_api/global/index.html).
pub fn new_logger(name: &str) -> ExporterBuilder {
    ExporterBuilder {
        provider_name: name.to_owned(),
        provider_id: Guid::from_name(name),
        provider_group: ProviderGroup::Unset,
        json: false,
        emit_common_schema_events: false,
        //exporter_config: None,
    }
}

impl ExporterBuilder {
    /// For advanced scenarios.
    /// Assign a provider ID to the ETW provider rather than use
    /// one generated from the provider name.
    pub fn with_provider_id(mut self, guid: Guid) -> Self {
        self.provider_id = guid;
        self
    }

    /// Get the current provider ID that will be used for the ETW provider.
    /// This is a convenience function to help with tools that do not implement
    /// the standard provider name to ID algorithm.
    pub fn get_provider_id(&self) -> Guid {
        self.provider_id
    }

    /// Override the default keywords and levels for events.
    /// Provide an implementation of the [`KeywordLevelProvider`] trait that will
    /// return the desired keywords and level values for each type of event.
    // pub fn with_custom_keywords_levels(
    //     mut self,
    //     config: impl KeywordLevelProvider + 'static,
    // ) -> Self {
    //     self.exporter_config = Some(Box::new(config));
    //     self
    // }

    /// For advanced scenarios.
    /// Encode the event payload as a single JSON string rather than multiple fields.
    /// Recommended only for compatibility with the C++ ETW exporter. In general,
    /// the textual representation of the event payload should be left to the event
    /// consumer.
    /// Requires the `json` feature to be enabled on the crate.
    #[cfg(any(feature = "json"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "json")))]
    pub fn with_json_payload(mut self) -> Self {
        self.json = true;
        self
    }

    /// For advanced scenarios.
    /// Emit extra events that follow the Common Schema 4.0 mapping.
    /// Recommended only for compatibility with specialized event consumers.
    /// Most ETW consumers will not benefit from events in this schema, and
    /// may perform worse.
    /// These events are emitted in addition to the normal ETW events,
    /// unless `without_realtime_events` is also called.
    /// Common Schema events are much slower to generate and should not be enabled
    /// unless absolutely necessary.
    pub fn with_common_schema_events(mut self) -> Self {
        self.emit_common_schema_events = true;
        self
    }

    /// For advanced scenarios.
    /// Set the ETW provider group to join this provider to.
    #[cfg(any(target_os = "windows", doc))]
    pub fn with_provider_group(mut self, group_id: Guid) -> Self {
        self.provider_group = ProviderGroup::Windows(group_id);
        self
    }

    /// For advanced scenarios.
    /// Set the EventHeader provider group to join this provider to.
    #[cfg(any(target_os = "linux", doc))]
    pub fn with_provider_group(mut self, name: &str) -> Self {
        self.provider_group = ProviderGroup::Linux(Cow::Owned(name.to_owned()));
        self
    }

    pub(crate) fn validate_config(&self) {
        match &self.provider_group {
            ProviderGroup::Unset => (),
            ProviderGroup::Windows(guid) => {
                assert_ne!(guid, &Guid::zero(), "Provider GUID must not be zeroes");
            }
            ProviderGroup::Linux(name) => {
                assert!(
                    eventheader_dynamic::ProviderOptions::is_valid_option_value(&name),
                    "Provider names must be lower case ASCII or numeric digits"
                );
            }
        }

        #[cfg(all(target_os = "linux"))]
        if self
            .provider_name
            .contains(|f: char| !f.is_ascii_alphanumeric())
        {
            // The perf command is very particular about the provider names it accepts.
            // The Linux kernel itself cares less, and other event consumers should also presumably not need this check.
            //panic!("Linux provider names must be ASCII alphanumeric");
        }
    }

    pub fn install(self) {
        self.validate_config();

        let _ = log::set_boxed_logger(Box::new(EtwEventHeaderLogger{}));
        log::set_max_level(log::LevelFilter::Trace);
    }
}

pub(crate) fn map_level(level: log::Level) -> u8 {
    match level {
        log::Level::Error => tracelogging::Level::Error.as_int(),
        log::Level::Warn => tracelogging::Level::Warning.as_int(),
        log::Level::Info => tracelogging::Level::Informational.as_int(),
        log::Level::Debug => tracelogging::Level::Verbose.as_int(),
        log::Level::Trace => tracelogging::Level::Verbose.as_int() + 1,
    }
}

fn create_provider(provider_name: &str) -> Pin<Arc<ProviderWrapper>> {
    let mut guard = PROVIDER_CACHE.write().unwrap();

    // Check again to see if it has already been created before we got the write lock
    if let Some(provider) = guard.get(provider_name) {
        provider.clone()
    } else {
        guard.insert(provider_name.to_string(), ProviderWrapper::new(provider_name, ProviderGroup::Unset));

        if let Some(provider) = guard.get(provider_name) {
            provider.clone()
        } else {
            panic!()
        }
    }
}

fn get_provider(provider_name: &str) -> Option<Pin<Arc<ProviderWrapper>>> {
    PROVIDER_CACHE.read().unwrap().get(provider_name).cloned()
}

fn get_or_create_provider(provider_name: &str) -> Pin<Arc<ProviderWrapper>> {
    if let Some(provider) = get_provider(provider_name) {
        provider
    } else {
        create_provider(provider_name)
    }
}

struct EtwEventHeaderLogger {}

impl EtwEventHeaderLogger {
    //pub fn new(provider_name)
}

impl Log for EtwEventHeaderLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let provider = get_or_create_provider(metadata.target());
        provider.enabled(map_level(metadata.level()), 0)
    }

    fn flush(&self) {
        
    }

    fn log(&self, record: &log::Record) {
        // Capture the current timestamp ASAP
        let timestamp = SystemTime::now();

        let provider = get_or_create_provider(record.target());
        provider.as_ref().write_record(timestamp, record);
    }
}

#[cfg(test)]
mod tests {
    use log::warn;

    use super::*;

    #[test]
    fn test1() {
        new_logger("MyDefaultProviderName").install();

        warn!(target: "MyRealProviderName", "My warning message");
    }
}

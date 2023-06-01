#[macro_use]
extern crate lazy_static;

mod etw;
pub mod logger;

#[cfg(feature="kv_unstable_json")]
pub mod event {
    use serde_derive::{Serialize, Deserialize};

    #[allow(non_camel_case_types)]
    #[derive(Serialize, Deserialize)]
    pub struct meta {
        pub provider: &'static str,
        pub event_name: &'static str,
        pub keyword: u64,
    }
}

#[macro_export]
macro_rules! evt_meta {
    ($provider:literal, $evtname:literal, $keyword:expr) => {
        log::kv::Value::capture_serde(&crate::event::meta{provider: $provider, event_name: $evtname, keyword: $keyword})
    };
}
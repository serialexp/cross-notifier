//! Calendar → notification pipeline.
//!
//! * [`source`] — where ICS data comes from (file, URL, CalDAV)
//! * [`parse`] — ICS → [`types::Occurrence`] list, with RRULE expansion
//! * [`store`] — JSON persistence for scheduler state
//! * [`notifier`] — how to deliver a reminder (in-process via CoreState)
//! * [`scheduler`] — holds pending fires, handles snooze/dismiss
//! * [`service`] — ties everything together, periodic refresh
//!
//! The service is designed to be embedded in either the standalone
//! `cross-notifier-server` or the desktop daemon — both already own a
//! `cross_notifier_core::CoreState`, which is what delivery needs.

pub mod notifier;
pub mod parse;
pub mod router;
pub mod scheduler;
pub mod service;
pub mod source;
pub mod store;
pub mod types;

pub use notifier::{CoreNotifier, CoreNotifierConfig, Notifier};
pub use router::{calendar_action_router, CalendarHandleSlot};
pub use scheduler::{SchedulerCmd, SchedulerHandle};
pub use service::{CalendarService, CalendarServiceConfig, DailySummaryConfig, dry_run};
pub use source::{CalDav, CalendarSource, IcsFile, IcsUrl};
pub use store::{JsonStore, MemoryStore, PendingMap, PendingStore};
pub use types::{EventInstance, Occurrence, PendingFire};

//! Source adapters. Each one runs on its own task or thread and reports
//! [`mv_core::SourceEvent`]s through an [`EventSender`].

pub mod bluos;
#[cfg(windows)]
pub mod smtc;

pub type EventSender = tokio::sync::mpsc::UnboundedSender<mv_core::SourceEvent>;

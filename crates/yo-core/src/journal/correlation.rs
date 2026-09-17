//! SessionJournal semantic correlation facade.

mod context;
mod records;

#[cfg(test)]
mod tests;

pub(crate) use context::ContextActiveSource;

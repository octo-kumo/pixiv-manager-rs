pub mod app;
pub mod bilibili;
pub mod cli;
pub mod core;
pub mod image;
pub mod io;
pub mod media;
pub mod onboarding_server;
pub mod profiling;
pub mod source;
pub mod theme;
pub mod twitter;
pub mod widgets;

#[cfg(feature = "test-util")]
pub mod test_util;

#[cfg(feature = "profiling")]
#[global_allocator]
static GLOBAL: profiling::memory::TaggedAllocator = profiling::memory::TaggedAllocator;

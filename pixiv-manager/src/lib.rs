// The wgpu resource graph (BindGroup → hub → registry) is deep; trait
// solving for the shader widget's Pipeline: MaybeSync needs the headroom.
#![recursion_limit = "256"]

pub mod app;
pub mod bilibili;
pub mod cli;
pub mod core;
pub mod frame;
pub mod image;
pub mod io;
pub mod media;
pub mod onboarding_server;
pub mod profiling;
pub mod source;
pub mod theme;
pub mod twitter;
pub mod ui;
pub mod viewer_tools;
pub mod widgets;

#[cfg(feature = "test-util")]
pub mod test_util;

#[cfg(feature = "profiling")]
#[global_allocator]
static GLOBAL: profiling::memory::TaggedAllocator = profiling::memory::TaggedAllocator;

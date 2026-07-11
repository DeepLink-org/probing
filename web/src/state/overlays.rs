//! Global app overlays (source viewer, sidebar monitors, …).

use dioxus::prelude::*;

use crate::state::scroll_lock::{lock_body_scroll, unlock_body_scroll};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceViewerTarget {
    pub path: String,
    pub line: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMonitor {
    Tasks,
    Overhead,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppOverlay {
    SourceViewer(SourceViewerTarget),
    Monitor(SidebarMonitor),
}

pub static APP_OVERLAY: GlobalSignal<Option<AppOverlay>> = Signal::global(|| None);

pub fn app_overlay() -> Option<AppOverlay> {
    APP_OVERLAY.read().clone()
}

pub fn open_app_overlay(overlay: AppOverlay) {
    *APP_OVERLAY.write() = Some(overlay);
    lock_body_scroll();
}

pub fn close_app_overlay() {
    *APP_OVERLAY.write() = None;
    unlock_body_scroll();
}

pub fn open_monitor_overlay(monitor: SidebarMonitor) {
    open_app_overlay(AppOverlay::Monitor(monitor));
}

pub fn monitor_overlay_open() -> Option<SidebarMonitor> {
    match app_overlay() {
        Some(AppOverlay::Monitor(m)) => Some(m),
        _ => None,
    }
}

pub fn open_source_viewer(path: String, line: Option<i64>) {
    open_app_overlay(AppOverlay::SourceViewer(SourceViewerTarget { path, line }));
}

pub fn close_source_viewer() {
    if matches!(app_overlay(), Some(AppOverlay::SourceViewer(_))) {
        close_app_overlay();
    }
}

pub fn source_viewer_open() -> bool {
    matches!(app_overlay(), Some(AppOverlay::SourceViewer(_)))
}

pub fn source_viewer_target() -> Option<SourceViewerTarget> {
    match app_overlay() {
        Some(AppOverlay::SourceViewer(target)) => Some(target),
        _ => None,
    }
}

mod analytics;
mod cluster;
mod dashboard;
mod distributed;
mod explore;
mod inference;
mod investigate;
mod memory;
mod profiles;
mod pulsing;
mod python;
mod rl;
mod stacks;
mod system;
mod tracing;
mod training;
mod training_placement;

pub use analytics::AnalyticsPage;
pub use cluster::ClusterPage;
pub use dashboard::DashboardPage;
pub use distributed::{DistributedPage, DistributedStatusPage};
pub use explore::{ExplorePage, NotFoundPage};
pub use inference::InferencePage;
pub use investigate::InvestigatePage;
pub(crate) use investigate::InvestigateSession;
pub use memory::MemoryPage;
pub use profiles::{ChromeTracePage, ProfileViewPage, ProfilesPage};
pub use pulsing::PulsingPage;
pub use python::PythonPage;
pub use rl::{PerfettoPage, ProcessTimelinePage, RlSpansPage, RlTrainPage, RolloutPage};
pub use stacks::{DistributedPythonStackPage, DistributedStackPage, StackPage, StackThreadPage};
pub use system::SystemPage;
pub use tracing::SpansPage;
pub use training::TrainingPage;

#[cfg(test)]
mod architecture_tests {
    const WORKSPACE_PAGES: &[(&str, &str)] = &[
        ("dashboard", include_str!("dashboard.rs")),
        ("cluster", include_str!("cluster.rs")),
        ("distributed", include_str!("distributed.rs")),
        ("training", include_str!("training.rs")),
        ("training placement", include_str!("training_placement.rs")),
        ("inference", include_str!("inference.rs")),
        ("memory", include_str!("memory.rs")),
        ("rl", include_str!("rl.rs")),
        ("profiles", include_str!("profiles.rs")),
        ("stacks", include_str!("stacks.rs")),
        ("tracing", include_str!("tracing.rs")),
        ("analytics", include_str!("analytics.rs")),
        ("pulsing", include_str!("pulsing.rs")),
        ("python", include_str!("python.rs")),
        ("system", include_str!("system.rs")),
        ("explore", include_str!("explore.rs")),
        ("investigate", include_str!("investigate.rs")),
    ];

    #[test]
    fn product_pages_share_the_next_workspace_frame() {
        for (name, source) in WORKSPACE_PAGES
            .iter()
            .filter(|(name, _)| *name != "training placement")
        {
            assert!(
                source.contains("WorkspacePage"),
                "{name} must use the canonical Next workspace frame"
            );
            assert!(
                !source.contains("NextPageHeader"),
                "{name} must not duplicate the Next page header"
            );
        }
    }

    #[test]
    fn next_diagnostic_text_does_not_drop_below_twelve_pixels() {
        let shared_sources = [
            ("components", include_str!("../components.rs")),
            ("shell", include_str!("../shell.rs")),
            ("sidebar", include_str!("../sidebar.rs")),
            (
                "span timeline",
                include_str!("../../components/span_timeline.rs"),
            ),
            (
                "call stack",
                include_str!("../../components/callstack_view.rs"),
            ),
        ];

        for (name, source) in WORKSPACE_PAGES.iter().copied().chain(shared_sources) {
            for size in [7, 8, 9, 10] {
                assert!(
                    !source.contains(&format!("text-[{size}px]")),
                    "{name} uses {size}px text for diagnostic UI"
                );
            }
        }
    }
}

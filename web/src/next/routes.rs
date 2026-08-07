use dioxus::prelude::*;
use dioxus_router::Routable;

use super::pages::{
    AnalyticsPage as Analytics, ChromeTracePage as ChromeTrace, ClusterPage as Cluster,
    DashboardPage as Dashboard, DistributedPage as Distributed,
    DistributedPythonStackPage as DistributedPythonStack, DistributedStackPage as DistributedStack,
    DistributedStatusPage as DistributedStatus, ExplorePage as Explore, InferencePage as Inference,
    InvestigatePage as Investigate, MemoryPage as Memory, NotFoundPage as NotFound,
    PerfettoPage as Perfetto, ProcessTimelinePage as ProcessTimeline,
    ProfileViewPage as ProfileView, ProfilesPage as Profiles, ProfilesPage as ProfilingLegacy,
    PulsingPage as Pulsing, PythonPage as Python, RlSpansPage as RlSpans, RlTrainPage as RlTrain,
    RolloutPage as Rollout, RolloutPage as RolloutLegacy, SpansPage as Spans,
    SpansPage as TracesLegacy, StackPage as Stack, StackThreadPage as StackThread,
    SystemPage as System, TrainingPage as Training,
};
use super::shell::NextShell;

#[derive(Routable, Clone, Debug, PartialEq)]
#[rustfmt::skip]
pub enum NextRoute {
    #[layout(NextShell)]
        #[route("/")]
        Dashboard {},

        #[route("/agent")]
        Investigate {},

        #[route("/training")]
        Training {},

        #[route("/rl")]
        Rollout {},

        #[route("/rl/rollout")]
        RolloutLegacy {},

        #[route("/rl/train")]
        RlTrain {},

        #[route("/rl/spans")]
        RlSpans {},

        #[route("/rl/process-timeline")]
        ProcessTimeline {},

        #[route("/rl/perfetto")]
        Perfetto {},

        #[route("/rl/inference")]
        Inference {},

        #[route("/distributed")]
        Distributed {},

        #[route("/cluster")]
        Cluster {},

        #[route("/cluster/status")]
        DistributedStatus {},

        #[route("/stacks")]
        Stack {},

        #[route("/stacks/distributed")]
        DistributedStack {},

        #[route("/stacks/distributed/py")]
        DistributedPythonStack {},

        #[route("/stacks/:tid")]
        StackThread { tid: String },

        #[route("/spans")]
        Spans {},

        #[route("/memory")]
        Memory {},

        #[route("/traces")]
        TracesLegacy {},

        #[route("/profiles")]
        Profiles {},

        #[route("/profiling")]
        ProfilingLegacy {},

        #[route("/profiling/:view")]
        ProfileView { view: String },

        #[route("/chrome-tracing")]
        ChromeTrace {},

        #[route("/analytics")]
        Analytics {},

        #[route("/python")]
        Python {},

        #[route("/pulsing")]
        Pulsing {},

        #[route("/system")]
        System {},

        #[route("/explore")]
        Explore {},

        #[route("/:..segments")]
        NotFound { segments: Vec<String> },
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn every_documented_product_path_resolves_in_next() {
        let paths = [
            "/",
            "/distributed",
            "/rl",
            "/rl/rollout",
            "/rl/train",
            "/rl/spans",
            "/rl/process-timeline",
            "/rl/perfetto",
            "/rl/inference",
            "/agent",
            "/cluster",
            "/cluster/status",
            "/stacks",
            "/stacks/distributed",
            "/stacks/distributed/py",
            "/stacks/123",
            "/profiling",
            "/profiles",
            "/profiling/pprof",
            "/analytics",
            "/python",
            "/traces",
            "/spans",
            "/memory",
            "/chrome-tracing",
            "/pulsing",
            "/training",
            "/system",
            "/explore",
        ];

        for path in paths {
            let route = NextRoute::from_str(path).unwrap_or_else(|error| {
                panic!("Next UI should resolve {path}: {error}");
            });
            assert!(
                !matches!(route, NextRoute::NotFound { .. }),
                "{path} unexpectedly resolved to the not-found route"
            );
        }
    }

    #[test]
    fn route_context_support_matches_actual_page_consumers() {
        assert!(NextRoute::Training {}.investigation_support().rank);
        assert!(!NextRoute::Training {}.investigation_support().step);
        assert!(NextRoute::Memory {}.investigation_support().device);
        assert!(NextRoute::Memory {}.investigation_support().host);
        assert!(NextRoute::Spans {}.investigation_support().trace);
        assert!(NextRoute::Spans {}.investigation_support().step);
        assert!(NextRoute::Cluster {}.investigation_support().host);
    }
}

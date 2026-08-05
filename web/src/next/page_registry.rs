//! Canonical metadata for every Next workspace.
//!
//! URL aliases belong in `routes.rs`; the rest of the shell consumes this
//! registry so compatibility names cannot drift into page behavior.

use super::routes::NextRoute;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InvestigationSupport {
    pub pid: bool,
    pub tid: bool,
    pub rank: bool,
    pub host: bool,
    pub device: bool,
    pub trace: bool,
    pub span: bool,
    pub step: bool,
}

impl InvestigationSupport {
    const fn all() -> Self {
        Self {
            pid: true,
            tid: true,
            rank: true,
            host: true,
            device: true,
            trace: true,
            span: true,
            step: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceKind {
    Standard,
    FullHeight,
}

impl WorkspaceKind {
    pub fn main_class(self) -> &'static str {
        match self {
            Self::Standard => "absolute inset-0 overflow-y-auto",
            Self::FullHeight => "absolute inset-0 overflow-hidden",
        }
    }

    pub fn content_class(self) -> &'static str {
        match self {
            Self::Standard => "mx-auto w-full max-w-[1600px] p-4 lg:p-5",
            Self::FullHeight => "h-full min-h-0 w-full p-4 lg:p-5",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTool {
    Dashboard,
    Investigate,
    Cluster,
    Training,
    Inference,
    Rl,
    Memory,
    Profiling,
    Stacks,
    Tracing,
    DeepTools,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub canonical_path: &'static str,
    pub description: &'static str,
    pub skills: &'static [&'static str],
    pub sidebar_group: &'static str,
    pub sidebar_title: &'static str,
    pub sidebar_tool: SidebarTool,
    pub workspace: WorkspaceKind,
    pub publishes_evidence: bool,
}

macro_rules! page {
    ($id:literal, $title:literal, $path:literal, $description:literal,
     $skills:expr, $group:literal, $sidebar_title:literal, $tool:ident) => {
        PageSpec {
            id: $id,
            title: $title,
            canonical_path: $path,
            description: $description,
            skills: $skills,
            sidebar_group: $group,
            sidebar_title: $sidebar_title,
            sidebar_tool: SidebarTool::$tool,
            workspace: WorkspaceKind::Standard,
            publishes_evidence: false,
        }
    };
}

impl NextRoute {
    pub fn page_spec(&self) -> PageSpec {
        let mut spec = match self {
            Self::Dashboard {} => page!(
                "dashboard",
                "Dashboard",
                "/",
                "Job progress, rank health, and local GPU utilization.",
                &["health_overview", "job_health", "slow_rank"],
                "Workspace",
                "Dashboard",
                Dashboard
            ),
            Self::Investigate {} => page!(
                "investigate",
                "Investigate",
                "/agent",
                "Skill-driven evidence collection and diagnostic reasoning.",
                &["health_overview"],
                "Workspace",
                "Investigate",
                Investigate
            ),
            Self::Training {} => page!(
                "training",
                "Training",
                "/training",
                "Step timing, placement, module hotspots, and collective latency.",
                &["slow_rank", "module_bottleneck"],
                "Workloads",
                "Training",
                Training
            ),
            Self::Rollout {} | Self::RolloutLegacy {} => page!(
                "rl-rollout",
                "RL Rollout",
                "/rl",
                "Per-trajectory phase timing across rollout workers.",
                &["health_overview", "module_bottleneck"],
                "Workloads",
                "RL",
                Rl
            ),
            Self::RlTrain {} => page!(
                "rl-train",
                "RL Train",
                "/rl/train",
                "Training batch phases keyed by train step.",
                &["slow_rank", "module_bottleneck"],
                "Workloads",
                "RL",
                Rl
            ),
            Self::RlSpans {} => page!(
                "rl-spans",
                "RL Spans",
                "/rl/spans",
                "Distributed RL span hierarchy with cross-process linking.",
                &["slow_rank", "comm_bottleneck"],
                "Workloads",
                "RL",
                Rl
            ),
            Self::ProcessTimeline {} => page!(
                "process-timeline",
                "Process Timeline",
                "/rl/process-timeline",
                "Per-process span timing and batch drill-down.",
                &["module_bottleneck"],
                "Workloads",
                "RL",
                Rl
            ),
            Self::Perfetto {} => page!(
                "perfetto",
                "Perfetto",
                "/rl/perfetto",
                "Chrome trace export for the loaded RL span set.",
                &["module_bottleneck", "comm_bottleneck"],
                "Workloads",
                "RL",
                Rl
            ),
            Self::Inference {} => page!(
                "inference",
                "Inference",
                "/rl/inference",
                "Inference engine throughput, latency, queue, and cache metrics.",
                &["gpu_pressure"],
                "Workloads",
                "Inference",
                Inference
            ),
            Self::Distributed {} => page!(
                "cluster-overview",
                "Cluster Overview",
                "/distributed",
                "Cluster completeness and latest comparable step duration by rank.",
                &["slow_rank", "nccl_culprit_victim"],
                "Workspace",
                "Cluster",
                Cluster
            ),
            Self::Cluster {} => page!(
                "cluster-nodes",
                "Cluster Nodes",
                "/cluster",
                "Registered nodes, roles, ranks, status, and heartbeat age.",
                &["job_health", "slow_rank"],
                "Workspace",
                "Cluster",
                Cluster
            ),
            Self::DistributedStatus {} => page!(
                "distributed-status",
                "Distributed Status",
                "/cluster/status",
                "PyTorch wait counters and read-only rendezvous store state.",
                &["training_hang", "comm_bottleneck"],
                "Workspace",
                "Cluster",
                Cluster
            ),
            Self::Stack {}
            | Self::StackThread { .. }
            | Self::DistributedStack {}
            | Self::DistributedPythonStack {} => page!(
                "stacks",
                "Stacks",
                "/stacks",
                "Local and distributed Python/native stack evidence.",
                &["training_hang", "module_bottleneck"],
                "Advanced analysis",
                "Stacks",
                Stacks
            ),
            Self::Spans {} | Self::TracesLegacy {} => page!(
                "spans",
                "Spans",
                "/spans",
                "Hierarchical spans, filters, attributes, and investigation context.",
                &["training_hang", "module_bottleneck"],
                "Advanced analysis",
                "Tracing",
                Tracing
            ),
            Self::Memory {} => page!(
                "memory",
                "Memory",
                "/memory",
                "Physical device capacity, sampled peaks, and allocator evidence.",
                &["gpu_pressure", "memory_leak"],
                "Advanced analysis",
                "Memory",
                Memory
            ),
            Self::Profiles {}
            | Self::ProfilingLegacy {}
            | Self::ProfileView { .. }
            | Self::ChromeTrace {} => page!(
                "profiles",
                "Profiling",
                "/profiles",
                "Current-process CPU, Torch, Chrome trace, PyTorch, and Ray capture evidence.",
                &["module_bottleneck", "comm_bottleneck"],
                "Advanced analysis",
                "Profiling",
                Profiling
            ),
            Self::Analytics {} => page!(
                "analytics",
                "SQL Explorer",
                "/analytics",
                "Local and federated table catalog, SQL editor, and results.",
                &["health_overview"],
                "Deep tools",
                "Toolbox",
                DeepTools
            ),
            Self::Python {} => page!(
                "python",
                "Python Trace",
                "/python",
                "Live function variable watches and historical records.",
                &["module_bottleneck"],
                "Deep tools",
                "Toolbox",
                DeepTools
            ),
            Self::Pulsing {} => page!(
                "pulsing",
                "Pulsing",
                "/pulsing",
                "Actor inventory, operation latency, explicit span errors, metrics, and membership.",
                &["health_overview"],
                "Deep tools",
                "Toolbox",
                DeepTools
            ),
            Self::System {} => page!(
                "system",
                "Process Snapshot",
                "/system",
                "Single-process identity, current resource samples, recent history, and CPU threads.",
                &["gpu_pressure", "module_bottleneck"],
                "Deep tools",
                "Toolbox",
                DeepTools
            ),
            Self::Explore {} | Self::ClassicFallback { .. } => page!(
                "explore",
                "Capability Catalog",
                "/explore",
                "Search canonical Next workspaces and recover unrecognized routes.",
                &[],
                "Deep tools",
                "Toolbox",
                DeepTools
            ),
        };

        if matches!(
            self,
            Self::Profiles {}
                | Self::ProfilingLegacy {}
                | Self::ProfileView { .. }
                | Self::ChromeTrace {}
                | Self::Perfetto {}
        ) {
            spec.workspace = WorkspaceKind::FullHeight;
        }
        spec.publishes_evidence = matches!(
            self,
            Self::Dashboard {} | Self::Training {} | Self::Memory {}
        );
        spec
    }

    /// Context fields that materially affect selection or filtering on this route.
    pub fn investigation_support(&self) -> InvestigationSupport {
        match self {
            Self::Dashboard {} => InvestigationSupport {
                rank: true,
                device: true,
                step: true,
                ..Default::default()
            },
            Self::Investigate {} => InvestigationSupport::all(),
            Self::Training {} => InvestigationSupport {
                rank: true,
                ..Default::default()
            },
            Self::Memory {} => InvestigationSupport {
                rank: true,
                host: true,
                device: true,
                ..Default::default()
            },
            Self::Spans {} | Self::TracesLegacy {} => InvestigationSupport {
                tid: true,
                rank: true,
                trace: true,
                span: true,
                step: true,
                ..Default::default()
            },
            Self::RlSpans {} => InvestigationSupport {
                tid: true,
                trace: true,
                span: true,
                ..Default::default()
            },
            Self::Cluster {} => InvestigationSupport {
                rank: true,
                host: true,
                device: true,
                ..Default::default()
            },
            Self::Stack {} | Self::StackThread { .. } => InvestigationSupport {
                tid: true,
                ..Default::default()
            },
            Self::Profiles {} | Self::ProfilingLegacy {} | Self::ProfileView { .. } => {
                InvestigationSupport {
                    tid: true,
                    ..Default::default()
                }
            }
            _ => InvestigationSupport::default(),
        }
    }

    pub fn snapshot_id(&self) -> &'static str {
        match self {
            Self::StackThread { .. } => "stack-thread",
            Self::DistributedStack {} => "distributed-stacks",
            Self::DistributedPythonStack {} => "distributed-python-stacks",
            Self::ProfileView { .. } => "profile-view",
            Self::ChromeTrace {} => "chrome-trace",
            _ => self.page_spec().id,
        }
    }

    pub fn uses_cluster_scope(&self) -> bool {
        matches!(
            self,
            Self::Dashboard {}
                | Self::Distributed {}
                | Self::Cluster {}
                | Self::DistributedStack {}
                | Self::DistributedPythonStack {}
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_routes_resolve_to_canonical_page_specs() {
        assert_eq!(
            NextRoute::RolloutLegacy {}.page_spec(),
            NextRoute::Rollout {}.page_spec()
        );
        assert_eq!(
            NextRoute::TracesLegacy {}.page_spec(),
            NextRoute::Spans {}.page_spec()
        );
        assert_eq!(
            NextRoute::ProfilingLegacy {}.page_spec(),
            NextRoute::Profiles {}.page_spec()
        );
    }

    #[test]
    fn runtime_status_keeps_local_scope_and_full_height_is_declared_once() {
        assert!(!NextRoute::DistributedStatus {}.uses_cluster_scope());
        assert_eq!(
            NextRoute::ProfileView {
                view: "trace".into()
            }
            .page_spec()
            .workspace,
            WorkspaceKind::FullHeight
        );
    }
}

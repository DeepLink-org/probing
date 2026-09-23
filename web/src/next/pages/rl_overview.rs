use dioxus::prelude::*;
use dioxus_router::{use_navigator, Link};
use probing_proto::prelude::{
    RlBenchmarkPoint, RlBenchmarksResponse, RlCompositionBucket, RlCompositionResponse,
    RlCompositionStep, RlDatasetRow, RlDatasetsResponse, RlEvent, RlPassBucket,
    RlPassHistogramResponse, RlRunSummary, RlSampleSummary, RlSamplerSnapshot, RlSeriesResponse,
    RlStalenessBucket, RlStalenessResponse,
};
use std::collections::BTreeMap;

use crate::api::ApiClient;
use crate::components::rl::metrics_line_chart::{
    ChartPoint, ChartSeries, ChartXAxis, MetricsLineChart,
};
use crate::hooks::{use_page_visible, use_polled_resource};
use crate::state::rl::SAMPLE_STEP_FILTER;
use crate::utils::error::Result;
use crate::utils::metric_format::{format_compact, format_metric_delta, format_metric_value};

use super::super::components::{
    EvidenceMetric, LoadingPanel, SectionCard, UnavailablePanel, WorkspacePage,
};
use super::super::rl_run::{read_selected_run_id, resolve_run, write_selected_run_id};
use super::super::routes::NextRoute;

const POLL_MS: u32 = 5_000;
/// Trend charts built on first paint. The grid is three wide at most, so this is
/// the top two rows, which is about what a normal window shows.
const EAGER_TREND_CHARTS: usize = 6;
/// Trend charts added per batch after the first paint.
const TREND_CHART_BATCH: usize = 3;
/// Pause between batches, long enough for the browser to paint and handle input
/// in between rather than building the whole grid in one frame.
const TREND_CHART_BATCH_MS: u32 = 120;
/// Placeholder height for a chart not yet built, matching what one occupies so
/// that filling it in does not shift the charts already on screen.
const PENDING_CHART_HEIGHT: f64 = 268.0;
/// Points per charted metric. The server buckets each series into this many,
/// so a chart spans the whole run no matter how long it gets; the row-limited
/// form shared its budget across all 18 names and truncated a long run to its
/// most recent steps.
const TREND_SERIES_BUCKETS: usize = 192;
/// Row budget for the full gauge list, which only needs the latest value and
/// the one before it to render a step-over-step change.
const LATEST_SERIES_LIMIT: usize = 3_000;
const SAMPLER_LIMIT: usize = 24;
const COMPOSITION_LIMIT: usize = 2_000;
const STALENESS_LIMIT: usize = 2_000;
const BENCHMARK_LIMIT: usize = 2_000;
const SAMPLE_QUEUE_LIMIT: usize = 24;
const EVENTS_LIMIT: usize = 40;
/// Rows per timeline column; enough to span a few restarts without scrolling.
const TIMELINE_LIMIT: usize = 24;
/// Widest step window the stacked composition chart renders.
const MAX_COMPOSITION_COLUMNS: usize = 48;
const PRIMARY_ACCENT: &str = "#2563eb";
const SECONDARY_ACCENT: &str = "#ea580c";

const OVERVIEW_METRICS: [(&str, &str, &str); 87] = [
    (
        "progress.total_steps",
        "Total steps",
        "Planned training steps for the whole run",
    ),
    (
        "progress.completed_steps",
        "Completed steps",
        "Training steps finished so far",
    ),
    (
        "progress.completed_ratio",
        "Completed share",
        "Finished share of the planned training steps",
    ),
    (
        "progress.eta_s",
        "Estimated time remaining",
        "Projected seconds remaining at the recent step rate",
    ),
    (
        "reward.mean",
        "Mean reward",
        "Mean reward over trajectories trained in one step",
    ),
    (
        "reward.raw_mean",
        "Mean raw reward",
        "Mean reward before shaping or normalization",
    ),
    (
        "reward.min",
        "Min reward",
        "Lowest reward in the trained batch",
    ),
    (
        "reward.max",
        "Max reward",
        "Highest reward in the trained batch",
    ),
    (
        "advantage.mean",
        "Mean advantage",
        "Mean advantage over the trained batch",
    ),
    (
        "advantage.min",
        "Min advantage",
        "Lowest advantage in the trained batch",
    ),
    (
        "advantage.max",
        "Max advantage",
        "Highest advantage in the trained batch",
    ),
    (
        "policy.entropy",
        "Policy entropy",
        "Mean per-token policy entropy",
    ),
    (
        "policy.entropy_rollout",
        "Rollout entropy",
        "Mean per-token entropy as measured by the sampler",
    ),
    (
        "policy.pg_loss",
        "Policy-gradient loss",
        "Policy-gradient loss",
    ),
    (
        "policy.base_loss",
        "Base loss",
        "Unweighted language-model loss before RL terms",
    ),
    (
        "policy.grad_norm",
        "Gradient norm",
        "Global gradient norm before clipping",
    ),
    (
        "policy.clip_frac_high",
        "Clip fraction (high)",
        "Share of tokens clipped at the upper ratio bound",
    ),
    (
        "policy.clip_frac_low",
        "Clip fraction (low)",
        "Share of tokens clipped at the lower ratio bound",
    ),
    (
        "policy.kl1",
        "Policy KL (k1)",
        "k1 estimator of KL against the reference policy",
    ),
    (
        "policy.kl3",
        "Policy KL (k3)",
        "k3 estimator of KL against the reference policy",
    ),
    (
        "policy.ratio_max",
        "Max policy ratio",
        "Largest importance ratio in the trained batch",
    ),
    (
        "policy.ratio_min",
        "Min policy ratio",
        "Smallest importance ratio in the trained batch",
    ),
    (
        "policy.ratio_abs_dev_mean",
        "Mean ratio deviation",
        "Mean absolute deviation of importance ratios from 1",
    ),
    (
        "policy.train_infer_kl",
        "Train / inference KL",
        "KL between rollout and trainer token distributions",
    ),
    (
        "policy.train_infer_k3_kl",
        "Train / inference KL (k3)",
        "k3 estimator of the rollout-versus-trainer KL",
    ),
    (
        "policy.train_infer_logprob_abs_diff",
        "Logprob gap (abs)",
        "Mean absolute per-token logprob gap between sampler and trainer",
    ),
    (
        "policy.train_infer_log_ppl_diff",
        "Log-ppl gap",
        "Log-perplexity gap between sampler and trainer",
    ),
    (
        "policy.train_infer_log_ppl_abs_diff",
        "Log-ppl gap (abs)",
        "Absolute log-perplexity gap between sampler and trainer",
    ),
    (
        "policy.train_infer_log_ppl_diff_max",
        "Log-ppl gap max",
        "Largest observed log-perplexity gap",
    ),
    (
        "policy.train_infer_log_ppl_diff_min",
        "Log-ppl gap min",
        "Smallest observed log-perplexity gap",
    ),
    (
        "policy.train_infer_ppl_ratio",
        "Perplexity ratio",
        "Trainer perplexity divided by sampler perplexity",
    ),
    (
        "policy.rollout_ppl",
        "Rollout perplexity",
        "Sequence perplexity as scored by the sampler",
    ),
    (
        "policy.rollout_log_ppl",
        "Rollout log-ppl",
        "Sequence log-perplexity as scored by the sampler",
    ),
    (
        "policy.training_ppl",
        "Training perplexity",
        "Sequence perplexity as scored by the trainer",
    ),
    (
        "policy.training_log_ppl",
        "Training log-ppl",
        "Sequence log-perplexity as scored by the trainer",
    ),
    (
        "context.prompt_tokens.mean",
        "Prompt tokens",
        "Mean prompt token count per trajectory",
    ),
    (
        "context.prompt_tokens.min",
        "Shortest prompt",
        "Shortest prompt in the batch, in tokens",
    ),
    (
        "context.prompt_tokens.max",
        "Longest prompt",
        "Longest prompt in the batch, in tokens",
    ),
    (
        "context.response_tokens.mean",
        "Response tokens",
        "Mean generated token count per trajectory",
    ),
    (
        "context.response_tokens.min",
        "Shortest response",
        "Shortest response in the batch, in tokens",
    ),
    (
        "context.response_tokens.max",
        "Longest response",
        "Longest response in the batch, in tokens",
    ),
    (
        "context.response_tokens.std",
        "Response length spread",
        "Standard deviation of response length in tokens",
    ),
    (
        "context.total_tokens.mean",
        "Total context tokens",
        "Mean prompt plus response token count",
    ),
    (
        "agent.turns.mean",
        "Agent turns",
        "Mean agent turns per trajectory",
    ),
    (
        "train.tokens",
        "Tokens trained",
        "Tokens trained in one step",
    ),
    (
        "train.img_tokens",
        "Image tokens",
        "Image tokens trained in one step",
    ),
    (
        "train.seqlen_tokens",
        "Sequence tokens",
        "Padded sequence capacity consumed in one step",
    ),
    (
        "train.efficient_attn_ratio",
        "Efficient attention share",
        "Share of attention compute spent on real tokens",
    ),
    (
        "train.img_efficient_attn_ratio",
        "Image attn share",
        "Image-token share of efficient attention compute",
    ),
    (
        "time.step_s",
        "Step time",
        "Whole-step wall-clock duration in seconds",
    ),
    (
        "time.rollout_s",
        "Rollout time",
        "Rollout generation wall-clock duration in seconds",
    ),
    (
        "time.training_s",
        "Training time",
        "Policy training wall-clock duration in seconds",
    ),
    (
        "time.onload_s",
        "Weight onload time",
        "Seconds spent loading weights onto the accelerator",
    ),
    (
        "time.offload_s",
        "Weight offload time",
        "Seconds spent offloading weights off the accelerator",
    ),
    (
        "time.switch_to_rollout_s",
        "Switch-to-rollout time",
        "Seconds spent handing weights to the sampler",
    ),
    (
        "time.save_ckpt_s",
        "Checkpoint save time",
        "Seconds spent writing a checkpoint",
    ),
    (
        "time.prepare_data_s",
        "Data prep time",
        "Seconds spent preparing the training batch",
    ),
    (
        "sampler.avg_pass",
        "Average pass rate",
        "Mean success fraction across prompt attempt groups",
    ),
    (
        "sampler.pass_zero_ratio",
        "No-pass prompt share",
        "Share of prompt groups with no successful attempt",
    ),
    (
        "sampler.pass_one_ratio",
        "All-pass prompt share",
        "Share of prompt groups with every attempt successful",
    ),
    (
        "sampler.infra_error_ratio",
        "Infrastructure error share",
        "Share of attempts lost to infrastructure failures",
    ),
    (
        "sampler.measurable_prompts",
        "Measurable prompts",
        "Prompt groups with a measurable pass rate",
    ),
    (
        "sampler.avg_staleness",
        "Average staleness",
        "Mean policy-version distance at training time",
    ),
    (
        "sampler.batch_size",
        "Rollout batch size",
        "Trajectories accepted into the trained batch",
    ),
    (
        "sampler.group_size",
        "Responses per prompt",
        "Responses sampled per prompt in the trained batch",
    ),
    (
        "sampler.task_count",
        "Rollout tasks",
        "Rollout tasks timed in the step",
    ),
    (
        "sampler.task_mean_s",
        "Mean task latency",
        "Mean rollout task duration in seconds",
    ),
    (
        "sampler.task_p50_s",
        "Task latency p50",
        "Median rollout task duration in seconds",
    ),
    (
        "sampler.task_p99_s",
        "Task latency p99",
        "99th-percentile rollout task duration in seconds",
    ),
    (
        "sampler.task_p99_p50_ratio",
        "Task straggler ratio",
        "Straggler spread, as p99 over p50 task duration",
    ),
    (
        "environment.active",
        "Active environments",
        "Active sandbox or environment count",
    ),
    (
        "environment.sandbox_total",
        "Sandbox total",
        "Cumulative sandbox or environment executions",
    ),
    (
        "environment.setup_error_ratio",
        "Env setup errors",
        "Share of environment setups that failed",
    ),
    (
        "environment.queue_s",
        "Env queue wait",
        "Mean environment queue wait in seconds",
    ),
    (
        "environment.leak_count",
        "Env leaks",
        "Environments still open after sample completion",
    ),
    (
        "throughput.e2e_samples_s",
        "End-to-end samples / s",
        "End-to-end trajectories trained per second",
    ),
    (
        "throughput.e2e_tokens_s",
        "End-to-end tokens / s",
        "End-to-end tokens trained per second",
    ),
    (
        "throughput.effective_samples_s",
        "Effective samples / s",
        "Trajectories per second excluding idle time",
    ),
    (
        "throughput.effective_tokens_s",
        "Effective tokens / s",
        "Tokens per second excluding idle time",
    ),
    (
        "throughput.training_tokens_s",
        "Training tokens / s",
        "Tokens per second during the training phase",
    ),
    (
        "throughput.rollout_samples_s",
        "Rollout samples / s",
        "Trajectories per second during rollout",
    ),
    (
        "throughput.rollout_tokens_s",
        "Rollout tokens / s",
        "Tokens per second during rollout",
    ),
    (
        "cost.usd_total",
        "Compute spend ($)",
        "Cumulative reported compute spend in USD",
    ),
    (
        "cost.usd_per_hour",
        "Spend rate ($/h)",
        "Recent compute spend rate in USD per hour",
    ),
    (
        "hardware.restart_count",
        "Hardware restarts",
        "Cumulative hardware or node restart count",
    ),
    (
        "hardware.max_memory_gb",
        "Peak GPU memory (GB)",
        "Peak accelerator memory allocated, in GiB",
    ),
    (
        "hardware.reserved_memory_gb",
        "Reserved GPU memory (GB)",
        "Accelerator memory reserved by the allocator, in GiB",
    ),
];
/// The trends worth a chart on the overview: reward and entropy to see whether
/// learning is happening, clipping and KL to see whether it is stable, timing
/// and throughput to see what it costs, and sampler health to see what is
/// getting thrown away.
const HEADLINE_METRICS: [&str; 18] = [
    "sampler.avg_pass",
    "reward.mean",
    "policy.entropy",
    "policy.pg_loss",
    "policy.grad_norm",
    "policy.train_infer_kl",
    "context.total_tokens.mean",
    "agent.turns.mean",
    "train.tokens",
    "time.step_s",
    "time.rollout_s",
    "time.training_s",
    "sampler.pass_zero_ratio",
    "sampler.pass_one_ratio",
    "sampler.infra_error_ratio",
    "environment.active",
    "sampler.avg_staleness",
    "sampler.measurable_prompts",
];
const ENV_METRICS: [(&str, &str); 5] = [
    ("environment.active", "Active"),
    ("environment.sandbox_total", "Sandboxes"),
    ("environment.setup_error_ratio", "Setup error"),
    ("environment.queue_s", "Queue wait"),
    ("environment.leak_count", "Leaks"),
];
const COLORS: [&str; 6] = [
    "#2563eb", "#dc2626", "#16a34a", "#9333ea", "#ea580c", "#0891b2",
];
const COMPOSITION_COLORS: [&str; 6] = [
    "#2563eb", "#16a34a", "#ea580c", "#9333ea", "#0891b2", "#64748b",
];

#[derive(Clone, Debug, PartialEq, Default)]
struct RunBroadcastData {
    run: RlRunSummary,
    sampler: Option<RlSamplerSnapshot>,
    /// Every overview gauge, but only a short tail: enough for the latest value
    /// and its step-over-step change.
    series: RlSeriesResponse,
    /// Only the charted metrics, summarised server-side into a fixed number of
    /// points. Requesting them raw shares one row budget with the gauges and
    /// truncates each chart to its most recent steps.
    trends: RlSeriesResponse,
    benchmarks: RlBenchmarksResponse,
    /// Kept per run so the timeline can column them; `RlEvent` carries no run id.
    events: Vec<RlEvent>,
}

#[derive(Clone, Debug, PartialEq)]
struct RlOverviewEvidence {
    runs: Vec<RlRunSummary>,
    primary_run_id: String,
    compare_run_id: Option<String>,
    primary: RunBroadcastData,
    secondary: Option<RunBroadcastData>,
    events: Vec<RlEvent>,
    sampler_history: Vec<RlSamplerSnapshot>,
    recent_samples: Vec<RlSampleSummary>,
    composition: RlCompositionResponse,
    composition_dimension: String,
    datasets: RlDatasetsResponse,
    pass_histogram: RlPassHistogramResponse,
    staleness: RlStalenessResponse,
}

#[component]
pub fn RlOverviewPage() -> Element {
    let visible = use_page_visible();
    let mut selected_run_id = use_signal(|| read_selected_run_id().unwrap_or_default());
    let mut compare_run_id = use_signal(String::new);
    let composition_dimension = use_signal(|| "task".to_string());
    let mut x_axis = use_signal(|| ChartXAxis::Step);
    // A long run needs more than one poll period to load, so a tick must not
    // cancel a load already under way.
    let evidence = use_polled_resource(POLL_MS, Some(visible), move || {
        let preferred = selected_run_id();
        let compare = compare_run_id();
        let dimension = composition_dimension();
        async move { load_overview(preferred, compare, dimension).await }
    });
    let state = evidence.read().clone();

    rsx! {
        WorkspacePage {
            title: "RL Overview".to_string(),
            subtitle: "MiMo-style dual-run broadcast with sampler funnel, environment health, and headline trainer trends.".to_string(),
            actions: rsx! {
                div { class: "flex items-center gap-2 text-xs text-gray-500",
                    span { "Live · {POLL_MS / 1000}s" }
                    select {
                        class: "rounded border border-gray-300 bg-white px-2 py-1 text-xs text-gray-700",
                        value: if matches!(x_axis(), ChartXAxis::Step) { "step" } else { "time" },
                        onchange: move |event| {
                            x_axis.set(if event.value() == "time" {
                                ChartXAxis::Time
                            } else {
                                ChartXAxis::Step
                            });
                        },
                        option { value: "step", "X: step" }
                        option { value: "time", "X: time" }
                    }
                }
            },
            match state {
                None => rsx! { LoadingPanel { label: "Loading RL overview".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "RL overview unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(evidence)) if evidence.primary.run.run_id.is_empty() => rsx! {
                    UnavailablePanel {
                        label: "No RL runs reported".to_string(),
                        detail: "Enable probing.ext.rl_data and wait for the first trainer step.".to_string(),
                    }
                },
                Some(Ok(evidence)) => rsx! {
                    OverviewWorkbench {
                        evidence,
                        selected_run_id,
                        compare_run_id,
                        composition_dimension,
                        x_axis: x_axis(),
                        on_select_run: move |run_id: String| {
                            write_selected_run_id(&run_id);
                            selected_run_id.set(run_id.clone());
                            if compare_run_id() == run_id {
                                compare_run_id.set(String::new());
                            }
                        },
                        on_select_compare: move |run_id: String| {
                            compare_run_id.set(run_id);
                        },
                    }
                },
            }
        }
    }
}

async fn load_overview(
    preferred: String,
    compare_preferred: String,
    dimension: String,
) -> Result<RlOverviewEvidence> {
    let client = ApiClient::new();
    let runs = client.fetch_rl_runs().await?.runs;
    let Some(primary_run) = resolve_run(&runs, Some(preferred.as_str())).cloned() else {
        return Ok(empty_evidence(runs, dimension));
    };
    let primary_run_id = primary_run.run_id.clone();
    let compare_id = resolve_compare_run(&runs, &primary_run_id, compare_preferred.as_str());
    // Every request below depends only on the run id, so issue them together.
    // Awaited one at a time they add up to more than the poll period on a long
    // run, which used to leave the page reloading forever.
    let secondary_id = compare_id
        .as_deref()
        .filter(|id| *id != primary_run_id.as_str());
    let (
        primary,
        secondary,
        sampler_history,
        recent_samples,
        composition,
        datasets,
        pass_histogram,
        staleness,
    ) = futures_util::join!(
        load_broadcast_data(&client, &primary_run_id),
        async {
            match secondary_id {
                Some(id) => load_broadcast_data(&client, id).await.map(Some),
                None => Ok(None),
            }
        },
        client.fetch_rl_sampler(&primary_run_id, SAMPLER_LIMIT),
        client.fetch_rl_samples(&primary_run_id, SAMPLE_QUEUE_LIMIT),
        client.fetch_rl_composition(&primary_run_id, &dimension, COMPOSITION_LIMIT),
        client.fetch_rl_datasets(&primary_run_id, COMPOSITION_LIMIT),
        client.fetch_rl_pass_histogram(&primary_run_id, COMPOSITION_LIMIT),
        client.fetch_rl_staleness(&primary_run_id, STALENESS_LIMIT),
    );
    let primary = primary?;
    let secondary = secondary?;
    let sampler_history = sampler_history?.snapshots;
    let recent_samples = recent_samples?.samples;
    let composition = composition?;
    let datasets = datasets?;
    let pass_histogram = pass_histogram?;
    let staleness = staleness?;
    let mut events = primary.events.clone();
    if let Some(secondary) = secondary.as_ref() {
        events.extend(secondary.events.clone());
    }
    events.sort_by(|left, right| {
        right
            .timestamp_ns
            .cmp(&left.timestamp_ns)
            .then_with(|| left.message.cmp(&right.message))
    });
    events.dedup_by(|left, right| left.message == right.message);
    events.truncate(EVENTS_LIMIT);
    Ok(RlOverviewEvidence {
        runs,
        primary_run_id,
        compare_run_id: compare_id,
        primary,
        secondary,
        events,
        sampler_history,
        recent_samples,
        composition,
        composition_dimension: dimension,
        datasets,
        pass_histogram,
        staleness,
    })
}

async fn load_broadcast(
    client: &ApiClient,
    run_id: &str,
) -> Result<(RlSeriesResponse, RlSeriesResponse, RlBenchmarksResponse)> {
    let names = OVERVIEW_METRICS
        .iter()
        .map(|(name, _, _)| *name)
        .collect::<Vec<_>>();
    let (series, trends, benchmarks) = futures_util::join!(
        client.fetch_rl_series(run_id, &names, LATEST_SERIES_LIMIT),
        client.fetch_rl_series_bucketed(run_id, &HEADLINE_METRICS, TREND_SERIES_BUCKETS),
        client.fetch_rl_benchmarks(run_id, BENCHMARK_LIMIT),
    );
    Ok((series?, trends?, benchmarks?))
}

async fn load_broadcast_data(client: &ApiClient, run_id: &str) -> Result<RunBroadcastData> {
    let (status, broadcast, events) = futures_util::join!(
        client.fetch_rl_status(run_id),
        load_broadcast(client, run_id),
        client.fetch_rl_events(run_id, EVENTS_LIMIT),
    );
    let status = status?;
    let (series, trends, benchmarks) = broadcast?;
    // A run without events is still worth broadcasting.
    let events = events.map(|response| response.events).unwrap_or_default();
    Ok(RunBroadcastData {
        run: status.run,
        sampler: status.sampler,
        series,
        trends,
        benchmarks,
        events,
    })
}

fn empty_evidence(runs: Vec<RlRunSummary>, dimension: String) -> RlOverviewEvidence {
    RlOverviewEvidence {
        runs,
        primary_run_id: String::new(),
        compare_run_id: None,
        primary: RunBroadcastData {
            run: RlRunSummary::default(),
            sampler: None,
            series: RlSeriesResponse::default(),
            trends: RlSeriesResponse::default(),
            benchmarks: RlBenchmarksResponse::default(),
            events: Vec::new(),
        },
        secondary: None,
        events: Vec::new(),
        sampler_history: Vec::new(),
        recent_samples: Vec::new(),
        datasets: RlDatasetsResponse::default(),
        pass_histogram: RlPassHistogramResponse::default(),
        composition: RlCompositionResponse::default(),
        composition_dimension: dimension,
        staleness: RlStalenessResponse::default(),
    }
}

fn resolve_compare_run(runs: &[RlRunSummary], primary_id: &str, preferred: &str) -> Option<String> {
    if !preferred.trim().is_empty()
        && preferred != primary_id
        && runs.iter().any(|run| run.run_id == preferred)
    {
        return Some(preferred.to_string());
    }
    default_secondary_run(runs, primary_id)
}

fn default_secondary_run(runs: &[RlRunSummary], primary_id: &str) -> Option<String> {
    let index = runs.iter().position(|run| run.run_id == primary_id)?;
    if let Some(next) = runs.get(index + 1) {
        if next.run_id != primary_id {
            return Some(next.run_id.clone());
        }
    }
    runs.iter()
        .find(|run| run.run_id != primary_id)
        .map(|run| run.run_id.clone())
}

#[component]
fn OverviewWorkbench(
    evidence: RlOverviewEvidence,
    selected_run_id: Signal<String>,
    compare_run_id: Signal<String>,
    composition_dimension: Signal<String>,
    x_axis: ChartXAxis,
    on_select_run: EventHandler<String>,
    on_select_compare: EventHandler<String>,
) -> Element {
    let navigator = use_navigator();
    let current = selected_run_id();
    let compare_value = compare_run_id();
    let dim = composition_dimension();
    // Display-only toggle, so it stays local instead of re-triggering the fetch.
    let mut composition_as_share = use_signal(|| true);
    // Trend charts are built a batch at a time. All eighteen at once is a few
    // thousand SVG nodes laid out before the page answers to input, and all but
    // the first rows start below the fold. Each batch yields to the event loop,
    // so scrolling and clicking stay live while the rest go up.
    let mut released_charts = use_signal(|| EAGER_TREND_CHARTS);
    use_future(move || async move {
        while *released_charts.peek() < HEADLINE_METRICS.len() {
            gloo_timers::future::TimeoutFuture::new(TREND_CHART_BATCH_MS).await;
            let next = (*released_charts.peek() + TREND_CHART_BATCH).min(HEADLINE_METRICS.len());
            released_charts.set(next);
        }
    });
    let primary = evidence.primary.clone();
    let latest_step = primary.run.global_step;
    let benchmark_charts = benchmark_series(&primary.benchmarks);
    // Runs in the same order as the column headers, primary first.
    let benchmark_runs = {
        let mut runs = vec![(evidence.primary_run_id.clone(), primary.benchmarks.clone())];
        if let Some(secondary) = evidence.secondary.as_ref() {
            runs.push((secondary.run.run_id.clone(), secondary.benchmarks.clone()));
        }
        runs
    };
    let benchmark_rows = benchmark_matrix(&benchmark_runs);
    let benchmark_run_ids = benchmark_runs
        .iter()
        .map(|(run_id, _)| run_id.clone())
        .collect::<Vec<_>>();
    let notices = evidence.events.clone();
    // Age notices against the newest telemetry rather than the browser clock, so
    // a skewed client does not report negative ages.
    let latest_telemetry_ns = evidence
        .runs
        .iter()
        .map(|run| run.timestamp_ns)
        .max()
        .unwrap_or(0);

    rsx! {
        if !notices.is_empty() {
            SectionCard {
                title: "Live notices".to_string(),
                subtitle: Some("From /apis/rl/events — operator notices, restarts, benchmark publishes, and sampler failures.".to_string()),
                div { class: "space-y-2 p-4",
                    for notice in notices {
                        div {
                            class: "flex items-baseline gap-3 rounded-md border px-3 py-2 text-xs {notice_level_class(&notice.level)}",
                            span { class: "w-16 shrink-0 font-mono opacity-60",
                                "{age_label(latest_telemetry_ns, notice.timestamp_ns)}"
                            }
                            span { class: "min-w-0 flex-1", "{notice.message}" }
                        }
                    }
                }
            }
        }

        SectionCard {
            title: "Run broadcast".to_string(),
            subtitle: Some(format!(
                "{} reported run(s) · compare {}",
                evidence.runs.len(),
                evidence
                    .compare_run_id
                    .as_deref()
                    .unwrap_or("auto (next run)")
            )),
            div { class: "grid gap-3 border-b border-gray-200 px-4 py-3 lg:grid-cols-2",
                label { class: "block text-xs text-gray-600",
                    span { class: "mb-1 block font-medium uppercase tracking-wide", "Primary run" }
                    select {
                        class: "w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm",
                        value: "{current}",
                        onchange: move |event| on_select_run.call(event.value()),
                        for run in evidence.runs.iter() {
                            option {
                                value: "{run.run_id}",
                                selected: run.run_id == current
                                    || (current.is_empty() && run.run_id == evidence.primary_run_id),
                                "{run.framework} · {run.run_id} · step {run.global_step}"
                            }
                        }
                    }
                }
                label { class: "block text-xs text-gray-600",
                    span { class: "mb-1 block font-medium uppercase tracking-wide", "Compare run" }
                    select {
                        class: "w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm",
                        value: "{compare_value}",
                        onchange: move |event| on_select_compare.call(event.value()),
                        option {
                            value: "",
                            selected: compare_value.is_empty(),
                            "Auto · next run in list"
                        }
                        for run in evidence.runs.iter().filter(|run| run.run_id != evidence.primary_run_id) {
                            option {
                                value: "{run.run_id}",
                                selected: compare_value == run.run_id,
                                "{run.framework} · {run.run_id} · step {run.global_step}"
                            }
                        }
                    }
                }
            }
            div { class: "grid grid-cols-1 gap-3 p-4 xl:grid-cols-2",
                RunBroadcastCard {
                    label: "Primary".to_string(),
                    accent: PRIMARY_ACCENT,
                    data: primary.clone(),
                }
                if let Some(secondary) = evidence.secondary.clone() {
                    RunBroadcastCard {
                        label: "Compare".to_string(),
                        accent: SECONDARY_ACCENT,
                        data: secondary,
                    }
                } else {
                    div { class: "flex items-center justify-center rounded-lg border border-dashed border-gray-300 bg-gray-50 px-4 py-10 text-xs text-gray-500",
                        "No secondary run selected."
                    }
                }
            }
            div { class: "border-t border-gray-200 px-4 py-2 flex flex-wrap items-center justify-end gap-3",
                button {
                    class: "text-xs font-medium text-blue-600 hover:underline",
                    onclick: move |_| {
                        *SAMPLE_STEP_FILTER.write() = Some(latest_step);
                        navigator.push(NextRoute::RlSamples {});
                    },
                    "Open samples at step {latest_step} →"
                }
                Link {
                    to: NextRoute::RlAbout {},
                    class: "text-xs font-medium text-blue-600 hover:underline",
                    "About this run →"
                }
                Link {
                    to: NextRoute::RlMetrics {},
                    class: "text-xs font-medium text-blue-600 hover:underline",
                    "Open metrics overlay →"
                }
                Link {
                    to: NextRoute::Rollout {},
                    class: "text-xs font-medium text-blue-600 hover:underline",
                    "Open rollout span drill-down →"
                }
            }
        }

        if !benchmark_rows.is_empty() {
            SectionCard {
                title: "Benchmark matrix".to_string(),
                subtitle: Some(
                    "Latest score per benchmark for each run being compared. Rows are only comparable when the harness matches."
                        .to_string(),
                ),
                BenchmarkMatrix {
                    run_ids: benchmark_run_ids.clone(),
                    rows: benchmark_rows.clone(),
                }
            }
        }

        if !benchmark_charts.is_empty() {
            SectionCard {
                title: "Benchmarks".to_string(),
                subtitle: Some("Offline evaluation scores by trainer step (primary run).".to_string()),
                div { class: "grid grid-cols-1 gap-3 xl:grid-cols-2",
                    for (name, caption, series) in benchmark_charts.into_iter() {
                        MetricsLineChart {
                            title: name,
                            series: vec![series],
                            x_axis,
                            height: 180.0,
                            subtitle: caption,
                            on_point_click: move |step| {
                                if step >= 0 {
                                    *SAMPLE_STEP_FILTER.write() = Some(step);
                                    navigator.push(NextRoute::RlSamples {});
                                }
                            },
                        }
                    }
                }
            }
        }

        if let Some(sampler) = primary.sampler.clone() {
            SectionCard {
                title: format!("Sampler · step {}", sampler.step),
                subtitle: Some(format!(
                    "Accepted {} / target {} · trained {}.",
                    sampler.accepted, sampler.target, sampler.trained
                )),
                div { class: "grid grid-cols-2 divide-x divide-gray-200 lg:grid-cols-4 xl:grid-cols-8",
                    EvidenceMetric { label: "Target".to_string(), value: sampler.target.to_string() }
                    EvidenceMetric { label: "Accepted".to_string(), value: sampler.accepted.to_string() }
                    EvidenceMetric { label: "Judged".to_string(), value: sampler.judged.to_string() }
                    EvidenceMetric { label: "Trained".to_string(), value: sampler.trained.to_string() }
                    EvidenceMetric { label: "Filtered".to_string(), value: sampler.filtered.to_string() }
                    EvidenceMetric { label: "Failed".to_string(), value: sampler.failed.to_string() }
                    EvidenceMetric { label: "Expired".to_string(), value: sampler.expired.to_string() }
                    EvidenceMetric { label: "In flight".to_string(), value: sampler.in_flight.to_string() }
                }
            }
        }

        if !evidence.recent_samples.is_empty() {
            SectionCard {
                title: "Sample queue".to_string(),
                subtitle: Some(format!(
                    "Latest {} rollout outcomes.",
                    evidence.recent_samples.len()
                )),
                div { class: "overflow-x-auto",
                    table { class: "min-w-full divide-y divide-gray-200 text-left text-xs",
                        thead { class: "bg-gray-50 text-[11px] uppercase tracking-wide text-gray-500",
                            tr {
                                th { class: "px-3 py-2 font-medium", "Step" }
                                th { class: "px-3 py-2 font-medium", "Task" }
                                th { class: "px-3 py-2 font-medium", "Status" }
                                th { class: "px-3 py-2 font-medium", "Reward" }
                                th { class: "px-3 py-2 font-medium", "Tokens" }
                                th { class: "px-3 py-2 font-medium", "Stale" }
                                th { class: "px-3 py-2 font-medium", "Rollout" }
                            }
                        }
                        tbody { class: "divide-y divide-gray-100 bg-white",
                            for sample in evidence.recent_samples.iter().take(12) {
                                {
                                    let reward = sample
                                        .reward
                                        .map(format_metric)
                                        .unwrap_or_else(|| "—".to_string());
                                    let tokens = sample.prompt_tokens + sample.response_tokens;
                                    let rollout = if sample.rollout_id.is_empty() {
                                        sample.sample_id.clone()
                                    } else {
                                        sample.rollout_id.clone()
                                    };
                                    rsx! {
                                        tr {
                                            td { class: "px-3 py-2 font-medium text-gray-800", "{sample.step}" }
                                            td { class: "px-3 py-2 text-gray-700", "{nonempty(&sample.task)}" }
                                            td { class: "px-3 py-2 text-gray-700", "{nonempty(&sample.status)}" }
                                            td { class: "px-3 py-2 text-gray-700", "{reward}" }
                                            td { class: "px-3 py-2 text-gray-700", "{tokens}" }
                                            td { class: "px-3 py-2 text-gray-700", "{sample.staleness}" }
                                            td { class: "px-3 py-2 font-mono text-[11px] text-gray-600", "{rollout}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if !evidence.sampler_history.is_empty() {
            SectionCard {
                title: "Sampler live feed".to_string(),
                subtitle: Some(format!(
                    "Recent {} funnel snapshots ordered by step.",
                    evidence.sampler_history.len()
                )),
                div { class: "overflow-x-auto",
                    table { class: "min-w-full divide-y divide-gray-200 text-left text-xs",
                        thead { class: "bg-gray-50 text-[11px] uppercase tracking-wide text-gray-500",
                            tr {
                                th { class: "px-3 py-2 font-medium", "Step" }
                                th { class: "px-3 py-2 font-medium", "Accepted" }
                                th { class: "px-3 py-2 font-medium", "Judged" }
                                th { class: "px-3 py-2 font-medium", "Trained" }
                                th { class: "px-3 py-2 font-medium", "Filtered" }
                                th { class: "px-3 py-2 font-medium", "Failed" }
                                th { class: "px-3 py-2 font-medium", "In flight" }
                                th { class: "px-3 py-2 font-medium", "Yield" }
                                th { class: "px-3 py-2 font-medium", "Samples" }
                            }
                        }
                        tbody { class: "divide-y divide-gray-100 bg-white",
                            for snapshot in evidence.sampler_history.iter().rev().take(6) {
                                {
                                    let step = snapshot.step;
                                    let yield_rate = if snapshot.accepted > 0 {
                                        format!(
                                            "{:.0}%",
                                            100.0 * snapshot.trained as f64 / snapshot.accepted as f64
                                        )
                                    } else {
                                        "—".to_string()
                                    };
                                    rsx! {
                                        tr {
                                            td { class: "px-3 py-2 font-medium text-gray-800", "{snapshot.step}" }
                                            td { class: "px-3 py-2 text-gray-700", "{snapshot.accepted}" }
                                            td { class: "px-3 py-2 text-gray-700", "{snapshot.judged}" }
                                            td { class: "px-3 py-2 text-gray-700", "{snapshot.trained}" }
                                            td { class: "px-3 py-2 text-gray-700", "{snapshot.filtered}" }
                                            td { class: "px-3 py-2 text-gray-700", "{snapshot.failed}" }
                                            td { class: "px-3 py-2 text-gray-700", "{snapshot.in_flight}" }
                                            td { class: "px-3 py-2 text-gray-700", "{yield_rate}" }
                                            td { class: "px-3 py-2",
                                                button {
                                                    class: "font-medium text-blue-600 hover:underline",
                                                    onclick: move |_| {
                                                        *SAMPLE_STEP_FILTER.write() = Some(step);
                                                        navigator.push(NextRoute::RlSamples {});
                                                    },
                                                    "Open →"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        SectionCard {
            title: "Environment health".to_string(),
            subtitle: Some("Sandbox and environment gauges from the primary adapter.".to_string()),
            div { class: "grid grid-cols-2 divide-x divide-gray-200 lg:grid-cols-5",
                for (name, label) in ENV_METRICS {
                    EvidenceMetric {
                        label: label.to_string(),
                        value: latest_value(&primary.series, name),
                    }
                }
            }
        }

        if !evidence.composition.buckets.is_empty() {
            SectionCard {
                title: "Batch composition".to_string(),
                subtitle: Some("What each step's samples are made of.".to_string()),
                div { class: "flex flex-wrap items-center gap-4 border-b border-gray-200 px-3 py-2 text-xs",
                    div { class: "flex items-center gap-2",
                        span { class: "text-gray-500", "by" }
                        select {
                            class: "rounded-md border border-gray-300 bg-white px-2 py-1",
                            value: "{dim}",
                            onchange: move |event| composition_dimension.set(event.value()),
                            option { value: "task", "data source" }
                            option { value: "category", "category" }
                            option { value: "status", "status" }
                            option { value: "filter_reason", "filter reason" }
                        }
                    }
                    div { class: "flex overflow-hidden rounded-md border border-gray-300",
                        button {
                            class: if composition_as_share() { "px-2 py-1 text-gray-600" } else { "bg-gray-800 px-2 py-1 text-white" },
                            onclick: move |_| composition_as_share.set(false),
                            "count"
                        }
                        button {
                            class: if composition_as_share() { "bg-gray-800 px-2 py-1 text-white" } else { "px-2 py-1 text-gray-600" },
                            onclick: move |_| composition_as_share.set(true),
                            "share"
                        }
                    }
                    span { class: "text-gray-500",
                        "{evidence.composition.buckets.len()} buckets · n={evidence.composition.sample_count}"
                    }
                }
                if let Some(latest) = evidence.composition.steps.last().cloned() {
                    div { class: "grid grid-cols-1 gap-3 xl:grid-cols-[2fr_1fr]",
                        div {
                            div { class: "px-4 pt-3 text-xs text-gray-500",
                                {composition_chart_caption(&evidence.composition, &dim)}
                            }
                            CompositionStepBars {
                                steps: evidence.composition.steps.clone(),
                                legend: composition_legend(&evidence.composition),
                                as_share: composition_as_share(),
                            }
                        }
                        div { class: "border-t border-gray-200 xl:border-l xl:border-t-0",
                            div { class: "px-4 py-3 text-xs font-medium text-gray-700",
                                "step {latest.step}"
                            }
                            CompositionStepTable {
                                latest,
                                previous: nth_from_last(&evidence.composition.steps, 1),
                                legend: composition_legend(&evidence.composition),
                                as_share: composition_as_share(),
                            }
                        }
                    }
                } else {
                    CompositionBars { buckets: evidence.composition.buckets.clone() }
                }
            }
        }

        if !evidence.datasets.datasets.is_empty() {
            SectionCard {
                title: "Data sources".to_string(),
                subtitle: Some(format!(
                    "Sampling outcome per data source over the last {} samples (through step {}).",
                    evidence.datasets.sample_count, evidence.datasets.latest_step
                )),
                DatasetTable { rows: evidence.datasets.datasets.clone() }
            }
        }

        if evidence.pass_histogram.prompt_count > 0 {
            SectionCard {
                title: "Pass rate distribution".to_string(),
                subtitle: Some(format!(
                    "How {} prompts split by pass rate across their rollouts · mean {:.1}% · {} rollouts. Ends are exact: leftmost is all-fail, rightmost all-pass.",
                    evidence.pass_histogram.prompt_count,
                    evidence.pass_histogram.avg_pass_rate * 100.0,
                    evidence.pass_histogram.rollout_count,
                )),
                PassHistogramBars { buckets: evidence.pass_histogram.buckets.clone() }
            }
        }

        if evidence.staleness.sample_count > 0 {
            SectionCard {
                title: "Staleness".to_string(),
                subtitle: Some(format!(
                    "avg {:.2} · n={}",
                    evidence.staleness.avg_staleness,
                    evidence.staleness.sample_count
                )),
                StalenessBars { buckets: evidence.staleness.buckets.clone() }
            }
        }

        SectionCard {
            title: "Latest metrics".to_string(),
            subtitle: Some("Canonical adapter values with step-over-step change (primary run).".to_string()),
            div { class: "grid grid-cols-3 gap-1.5 p-2 lg:grid-cols-6 xl:grid-cols-7",
                for (name, label, description) in OVERVIEW_METRICS {
                    div {
                        class: "rounded border border-gray-200 bg-gray-50 px-2 py-1.5",
                        title: "{name} — {description}",
                        EvidenceMetric {
                            label: label.to_string(),
                            value: latest_value(&primary.series, name),
                            detail: step_delta(&primary.series, name),
                        }
                    }
                }
            }
        }

        SectionCard {
            title: "Headline trends".to_string(),
            subtitle: Some(trends_subtitle(&primary)),
            div { class: "grid grid-cols-1 gap-3 xl:grid-cols-2 2xl:grid-cols-3",
                for (index, name) in HEADLINE_METRICS.iter().enumerate() {
                    {
                        let chart = rsx! {
                            MetricsLineChart {
                                title: metric_label(name).to_string(),
                                series: chart_series(
                                    &primary.trends,
                                    name,
                                    COLORS[index % COLORS.len()],
                                    x_axis,
                                ),
                                x_axis,
                                height: 180.0,
                                subtitle: series_stats(&primary.trends, name),
                                tooltip: metric_tooltip(name),
                                on_point_click: move |step| {
                                    if step >= 0 {
                                        *SAMPLE_STEP_FILTER.write() = Some(step);
                                        navigator.push(NextRoute::RlSamples {});
                                    }
                                },
                            }
                        };
                        rsx! {
                            if index < released_charts() {
                                {chart}
                            } else {
                                // Holds the row's height so that filling this in does
                                // not shift the charts already on screen.
                                div { style: "min-height: {PENDING_CHART_HEIGHT}px" }
                            }
                        }
                    }
                }
            }
        }

        SectionCard {
            title: "Timeline".to_string(),
            subtitle: Some("Step progress interleaved with reported events, most recent first.".to_string()),
            div { class: "grid grid-cols-1 gap-3 p-3 xl:grid-cols-2",
                RunTimeline {
                    run: primary.run.clone(),
                    rows: timeline_rows(&primary, TIMELINE_LIMIT),
                    color: COLORS[0],
                }
                if let Some(secondary) = evidence.secondary.as_ref() {
                    RunTimeline {
                        run: secondary.run.clone(),
                        rows: timeline_rows(secondary, TIMELINE_LIMIT),
                        color: COLORS[1],
                    }
                }
            }
        }
    }
}

#[component]
fn RunBroadcastCard(label: String, accent: &'static str, data: RunBroadcastData) -> Element {
    let run = data.run.clone();
    let spend = format_money(latest_numeric(&data.series, "cost.usd_total"));
    let spend_rate = format_money(latest_numeric(&data.series, "cost.usd_per_hour"));
    let pass_delta = delta_vs_first(&data.series, "sampler.avg_pass");
    let pass_value = format!(
        "{}{}",
        latest_value(&data.series, "sampler.avg_pass"),
        pass_delta
    );
    let step_tokens = latest_value(&data.series, "train.tokens");
    let throughput = latest_value(&data.series, "throughput.e2e_tokens_s");
    let cumulative_tokens = format_count(run.tokens_total);
    let sandboxes = latest_value(&data.series, "environment.sandbox_total");
    let restarts = latest_numeric(&data.series, "hardware.restart_count")
        .map(format_metric)
        .unwrap_or_else(|| "0".to_string());
    let pinned = pinned_benchmark(&latest_benchmarks(&data.benchmarks))
        .map(|(name, score, step)| format!("{name} · {} · step {step}", format_metric(score)))
        .unwrap_or_else(|| "—".to_string());
    let batch_shape = batch_shape_label(&data.series);
    let runtime = runtime_label(&run);
    let progress = progress_label(&data.series, run.global_step);
    let border_style = format!("border-color: {accent};");

    rsx! {
        div {
            class: "overflow-hidden rounded-lg border-2 bg-white shadow-sm",
            style: "{border_style}",
            div {
                class: "px-4 py-2 text-xs font-semibold uppercase tracking-wide text-white",
                style: "background-color: {accent};",
                "{label} · {run.framework} · {run.run_id}"
            }
            div { class: "grid grid-cols-2 gap-px bg-gray-200 sm:grid-cols-3",
                div { class: "bg-white px-3 py-3",
                    EvidenceMetric {
                        label: "Spend".to_string(),
                        value: spend,
                        detail: Some(format!("{spend_rate} / h")),
                    }
                }
                div { class: "bg-white px-3 py-3",
                    EvidenceMetric {
                        label: "Pass rate".to_string(),
                        value: pass_value,
                    }
                }
                div { class: "bg-white px-3 py-3",
                    EvidenceMetric {
                        label: format!("Tokens · step {}", run.global_step),
                        value: step_tokens,
                        detail: Some(format!("{throughput} tok/s")),
                    }
                }
                div { class: "bg-white px-3 py-3",
                    EvidenceMetric {
                        label: "Tokens · total".to_string(),
                        value: cumulative_tokens,
                        detail: Some(format!("{} samples", format_count(run.samples_total))),
                    }
                }
                div { class: "bg-white px-3 py-3",
                    EvidenceMetric {
                        label: "Train batch".to_string(),
                        value: batch_shape,
                    }
                }
                div { class: "bg-white px-3 py-3",
                    EvidenceMetric {
                        label: "Sandboxes".to_string(),
                        value: sandboxes,
                    }
                }
                div { class: "bg-white px-3 py-3",
                    EvidenceMetric {
                        label: "Restarts".to_string(),
                        value: restarts,
                    }
                }
                div { class: "bg-white px-3 py-3 sm:col-span-1",
                    EvidenceMetric {
                        label: "Pinned eval".to_string(),
                        value: pinned,
                    }
                }
            }
            div { class: "border-t border-gray-100 px-4 py-2 text-xs text-gray-500",
                "{progress} · {nonempty(&run.phase)} · {runtime}"
            }
        }
    }
}

#[component]
fn CompositionBars(buckets: Vec<RlCompositionBucket>) -> Element {
    rsx! {
        div { class: "space-y-3 px-4 py-4",
            div { class: "flex h-4 overflow-hidden rounded-full bg-gray-100",
                for (index, bucket) in buckets.iter().enumerate() {
                    {
                        let width = format!("{:.2}%", bucket.share * 100.0);
                        let color = COMPOSITION_COLORS[index % COMPOSITION_COLORS.len()];
                        rsx! {
                            div {
                                class: "h-full",
                                style: "width: {width}; background-color: {color};",
                                title: "{bucket.key}: {bucket.count}",
                            }
                        }
                    }
                }
            }
            div { class: "grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3",
                for (index, bucket) in buckets.iter().enumerate() {
                    {
                        let color = COMPOSITION_COLORS[index % COMPOSITION_COLORS.len()];
                        let share = format!("{:.1}%", bucket.share * 100.0);
                        rsx! {
                            div { class: "flex items-center justify-between gap-3 rounded-md border border-gray-200 px-3 py-2 text-xs",
                                div { class: "flex items-center gap-2 min-w-0",
                                    span {
                                        class: "inline-block h-2.5 w-2.5 shrink-0 rounded-full",
                                        style: "background-color: {color};",
                                    }
                                    span { class: "truncate font-medium text-gray-800", "{bucket.key}" }
                                }
                                span { class: "shrink-0 text-gray-600", "{bucket.count} · {share}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Stacked per-step bars: one column per trainer step, segments in legend order.
#[component]
fn CompositionStepBars(
    steps: Vec<RlCompositionStep>,
    legend: Vec<String>,
    as_share: bool,
) -> Element {
    // Past a few dozen columns the bars get too thin to compare, so show the
    // recent window rather than squeezing the whole run in.
    let visible = steps
        .len()
        .saturating_sub(MAX_COMPOSITION_COLUMNS)
        .min(steps.len());
    let steps = &steps[visible..];
    let max_total = steps
        .iter()
        .map(|step| step.sample_count)
        .max()
        .unwrap_or(1)
        .max(1);
    let widest_step = steps.iter().map(|step| step.step).max().unwrap_or(0);
    let label_every = composition_label_stride(steps.len(), widest_step);
    let last_column = steps.len().saturating_sub(1);
    rsx! {
        div { class: "space-y-2 px-4 py-4",
            div { class: "flex items-end gap-px h-40",
                for step in steps.iter() {
                    {
                        let column_height = if as_share {
                            100.0
                        } else {
                            100.0 * step.sample_count as f64 / max_total as f64
                        };
                        let denominator = step.sample_count.max(1) as f64;
                        rsx! {
                            div {
                                class: "flex h-full flex-1 flex-col justify-end",
                                title: "step {step.step} · n={step.sample_count}",
                                div {
                                    class: "flex w-full flex-col-reverse overflow-hidden rounded-t",
                                    style: "height: {column_height:.2}%;",
                                    for key in legend.iter() {
                                        {
                                            let count = step
                                                .buckets
                                                .iter()
                                                .find(|bucket| &bucket.key == key)
                                                .map(|bucket| bucket.count)
                                                .unwrap_or(0);
                                            let segment = 100.0 * count as f64 / denominator;
                                            let color = composition_color(&legend, key);
                                            rsx! {
                                                if count > 0 {
                                                    div {
                                                        class: "w-full",
                                                        style: "height: {segment:.2}%; background-color: {color};",
                                                        title: "{key}: {count}",
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Axis lives outside the fixed-height bar row: when the labels shared
            // that row, a labelled column lost ~15px of bar and the numbers ended
            // up printed over the bars.
            div { class: "flex gap-px",
                for (column, step) in steps.iter().enumerate() {
                    div { class: "flex-1 text-center text-[11px] leading-4 text-gray-500",
                        if composition_shows_label(column, last_column, label_every) {
                            "{step.step}"
                        }
                    }
                }
            }
            div { class: "flex flex-wrap gap-x-4 gap-y-1",
                for key in legend.iter() {
                    {
                        let color = composition_color(&legend, key);
                        rsx! {
                            div { class: "flex items-center gap-1.5 text-[11px] text-gray-700",
                                span {
                                    class: "inline-block h-2 w-2 rounded-full",
                                    style: "background-color: {color};",
                                }
                                "{key}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Latest step's breakdown with the change in share against the previous step.
#[component]
/// Benchmarks down the rows and runs across the columns, for comparing two
/// versions of a model on the same evaluations.
#[component]
fn BenchmarkMatrix(run_ids: Vec<String>, rows: Vec<BenchmarkMatrixRow>) -> Element {
    let show_gap = run_ids.len() >= 2;
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "min-w-full divide-y divide-gray-200 text-xs",
                thead { class: "bg-gray-50 text-left text-gray-500",
                    tr {
                        th { class: "px-3 py-2 font-medium", "benchmark" }
                        for run_id in run_ids.iter() {
                            th { class: "px-3 py-2 text-right font-medium", "{run_id}" }
                        }
                        if show_gap {
                            th { class: "px-3 py-2 text-right font-medium", "gap" }
                        }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for row in rows.iter() {
                        tr {
                            td { class: "px-3 py-2",
                                div { class: "font-medium text-gray-800", "{row.name}" }
                                div { class: "text-[11px] text-gray-400", "{row.caption}" }
                            }
                            for cell in row.cells.iter() {
                                td { class: "px-3 py-2 text-right",
                                    if let Some(score) = cell.score {
                                        div { class: "text-gray-800",
                                            {format_metric_value("benchmark.score", score)}
                                        }
                                        div { class: "text-[11px] text-gray-400",
                                            "step {cell.step} · {cell.delta}"
                                        }
                                    } else {
                                        span { class: "text-gray-300", "—" }
                                    }
                                }
                            }
                            if show_gap {
                                td { class: "px-3 py-2 text-right text-gray-600",
                                    {benchmark_gap(row)}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Sampling funnel per data source, so a dataset stuck in judging or losing
/// everything to filters is visible without a SQL query.
#[component]
fn DatasetTable(rows: Vec<RlDatasetRow>) -> Element {
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "min-w-full divide-y divide-gray-200 text-xs",
                thead { class: "bg-gray-50 text-left text-gray-500",
                    tr {
                        th { class: "px-3 py-2 font-medium", "data source" }
                        th { class: "px-3 py-2 font-medium", "category" }
                        th { class: "px-3 py-2 text-right font-medium", "samples" }
                        th { class: "px-3 py-2 text-right font-medium", "completed" }
                        th { class: "px-3 py-2 text-right font-medium", "filtered" }
                        th { class: "px-3 py-2 text-right font-medium", "failed" }
                        th { class: "px-3 py-2 text-right font-medium", "in flight" }
                        th { class: "px-3 py-2 text-right font-medium", "pass rate" }
                        th { class: "px-3 py-2 text-right font-medium", "last step" }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for row in rows.iter() {
                        tr {
                            td { class: "px-3 py-2 font-medium text-gray-800", "{row.task}" }
                            td { class: "px-3 py-2 text-gray-500", "{row.category}" }
                            td { class: "px-3 py-2 text-right text-gray-700", "{row.samples}" }
                            td { class: "px-3 py-2 text-right text-gray-700", "{row.completed}" }
                            td {
                                class: if row.filtered > 0 { "px-3 py-2 text-right text-amber-700" } else { "px-3 py-2 text-right text-gray-400" },
                                "{row.filtered}"
                            }
                            td {
                                class: if row.failed > 0 { "px-3 py-2 text-right text-red-700" } else { "px-3 py-2 text-right text-gray-400" },
                                "{row.failed}"
                            }
                            td {
                                class: if row.in_flight > 0 { "px-3 py-2 text-right text-blue-700" } else { "px-3 py-2 text-right text-gray-400" },
                                "{row.in_flight}"
                            }
                            td { class: "px-3 py-2 text-right text-gray-700",
                                {pass_rate_label(row.pass_rate)}
                            }
                            td { class: "px-3 py-2 text-right text-gray-500", "{row.last_step}" }
                        }
                    }
                }
            }
        }
    }
}

/// Renders a missing pass rate as an em dash rather than a misleading `0.0 %`.
fn pass_rate_label(pass_rate: Option<f64>) -> String {
    match pass_rate {
        Some(rate) => format!("{:.1} %", rate * 100.0),
        None => "—".to_string(),
    }
}

#[component]
fn CompositionStepTable(
    latest: RlCompositionStep,
    previous: Option<RlCompositionStep>,
    legend: Vec<String>,
    as_share: bool,
) -> Element {
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "min-w-full divide-y divide-gray-200 text-xs",
                thead { class: "bg-gray-50 text-left text-gray-500",
                    tr {
                        th { class: "px-3 py-2 font-medium", "bucket" }
                        th { class: "px-3 py-2 font-medium", if as_share { "share" } else { "samples" } }
                        th { class: "px-3 py-2 font-medium", "Δ vs prev step" }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for bucket in latest.buckets.iter() {
                        {
                            let color = composition_color(&legend, &bucket.key);
                            let value = if as_share {
                                format!("{:.1}%", bucket.share * 100.0)
                            } else {
                                bucket.count.to_string()
                            };
                            let delta = share_delta(previous.as_ref(), &bucket.key, bucket.share);
                            rsx! {
                                tr {
                                    td { class: "px-3 py-2",
                                        div { class: "flex items-center gap-2",
                                            span {
                                                class: "inline-block h-2 w-2 shrink-0 rounded-full",
                                                style: "background-color: {color};",
                                            }
                                            span { class: "font-medium text-gray-800", "{bucket.key}" }
                                        }
                                    }
                                    td { class: "px-3 py-2 text-gray-700", "{value}" }
                                    td { class: "px-3 py-2 text-gray-500", "{delta}" }
                                }
                            }
                        }
                    }
                    tr { class: "bg-gray-50 font-medium text-gray-800",
                        td { class: "px-3 py-2", "total" }
                        td { class: "px-3 py-2",
                            if as_share { "100%" } else { "{latest.sample_count}" }
                        }
                        td { class: "px-3 py-2 text-gray-500", "n={latest.sample_count}" }
                    }
                }
            }
        }
    }
}

/// `1568 × 16 seqs`, or just the batch size when the group size is unreported.
fn batch_shape_label(series: &RlSeriesResponse) -> String {
    let Some(batch) = latest_numeric(series, "sampler.batch_size") else {
        return "—".to_string();
    };
    match latest_numeric(series, "sampler.group_size") {
        Some(group) if group >= 1.0 => format!("{batch:.0} × {group:.0} seqs"),
        _ => format!("{batch:.0}"),
    }
}

/// One run's timeline column.
#[component]
fn RunTimeline(run: RlRunSummary, rows: Vec<TimelineRow>, color: &'static str) -> Element {
    rsx! {
        div { class: "rounded-lg border border-gray-200 bg-white",
            div { class: "flex items-center justify-between gap-2 border-b border-gray-200 px-3 py-2",
                div { class: "flex min-w-0 items-center gap-2",
                    span {
                        class: "inline-block h-2 w-2 shrink-0 rounded-full",
                        style: "background-color: {color};",
                    }
                    span { class: "truncate text-sm font-medium text-gray-800", "{run.run_id}" }
                }
                span { class: "shrink-0 text-xs text-gray-500", "{run_phase_label(&run)}" }
            }
            if rows.is_empty() {
                div { class: "px-3 py-6 text-center text-xs text-gray-400", "No events yet" }
            } else {
                div { class: "divide-y divide-gray-100",
                    for row in rows.iter() {
                        div { class: "flex items-baseline gap-3 px-3 py-1.5 text-xs",
                            span { class: "w-24 shrink-0 font-mono text-gray-400", "{row.offset}" }
                            span { class: "min-w-0 flex-1 {timeline_level_class(&row.level)}",
                                "{row.headline}"
                            }
                            span { class: "shrink-0 font-mono text-gray-500", "{row.trailing}" }
                        }
                    }
                }
            }
        }
    }
}

fn timeline_level_class(level: &str) -> &'static str {
    match level {
        "error" => "text-red-700",
        "warning" => "text-amber-700",
        _ => "text-gray-700",
    }
}

fn run_phase_label(run: &RlRunSummary) -> String {
    if run.phase.is_empty() {
        "running".to_string()
    } else {
        run.phase.clone()
    }
}

/// One line in a run's timeline: either a step's progress or a reported event.
#[derive(Clone, Debug, PartialEq)]
struct TimelineRow {
    timestamp_ns: i64,
    /// Offset from the run's start, as `T+5d 07:14`.
    offset: String,
    headline: String,
    /// Right-aligned secondary fact, such as the step's token count.
    trailing: String,
    level: String,
}

/// Interleave step progress with reported events, most recent first.
///
/// Step rows carry the headline metric and token count for that step, so the
/// timeline answers "what was happening when it restarted" without leaving it.
fn timeline_rows(data: &RunBroadcastData, limit: usize) -> Vec<TimelineRow> {
    let start = data.run.start_time_ns;
    let headline = "sampler.avg_pass";
    let mut rows = Vec::new();
    // Trends, not the gauge tail: the timeline needs one row per step.
    if let Some(points) = data.trends.series.get(headline) {
        let tokens = data.trends.series.get("train.tokens");
        for (index, point) in points.iter().enumerate() {
            let value = format_metric_value(headline, point.value);
            let delta = index
                .checked_sub(1)
                .map(|previous| format_metric_delta(headline, point.value - points[previous].value))
                .unwrap_or_default();
            let step_tokens = tokens
                .and_then(|series| series.iter().find(|entry| entry.step == point.step))
                .map(|entry| format!("{} tok", format_compact(entry.value)))
                .unwrap_or_default();
            rows.push(TimelineRow {
                timestamp_ns: point.timestamp_ns,
                offset: offset_label(start, point.timestamp_ns),
                headline: format!("step {} · {headline} {value} {delta}", point.step),
                trailing: step_tokens,
                level: "info".to_string(),
            });
        }
    }
    for event in &data.events {
        rows.push(TimelineRow {
            timestamp_ns: event.timestamp_ns,
            offset: offset_label(start, event.timestamp_ns),
            headline: event.message.clone(),
            trailing: if event.step >= 0 {
                format!("step {}", event.step)
            } else {
                String::new()
            },
            level: event.level.clone(),
        });
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.timestamp_ns));
    rows.truncate(limit);
    rows
}

/// How long before `reference` something happened, as `1d 07h` or `12m`.
fn age_label(reference_ns: i64, timestamp_ns: i64) -> String {
    if reference_ns <= 0 || timestamp_ns <= 0 {
        return String::new();
    }
    let seconds = (reference_ns - timestamp_ns).max(0) / 1_000_000_000;
    if seconds >= 86_400 {
        format!("{}d {:02}h", seconds / 86_400, seconds % 86_400 / 3_600)
    } else if seconds >= 3_600 {
        format!("{}h {:02}m", seconds / 3_600, seconds % 3_600 / 60)
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        "just now".to_string()
    }
}

/// `T+5d 07:14` into a long run, or `T+04:12` within the first hour.
fn offset_label(start_ns: i64, timestamp_ns: i64) -> String {
    if start_ns <= 0 || timestamp_ns <= 0 || timestamp_ns < start_ns {
        return String::new();
    }
    let seconds = (timestamp_ns - start_ns) / 1_000_000_000;
    if seconds < 3_600 {
        return format!("T+{:02}:{:02}", seconds / 60, seconds % 60);
    }
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    format!("T+{days}d {hours:02}:{minutes:02}")
}

/// Space the axis by how wide the step numbers actually are: a five-digit step
/// needs more columns between labels than a two-digit one to stay legible.
fn composition_label_stride(columns: usize, widest_step: i64) -> usize {
    let digits = widest_step.abs().to_string().len();
    let target = match digits {
        0..=3 => 15,
        4 => 12,
        _ => 9,
    };
    (columns / target).max(1)
}

/// Labels are counted back from the newest step, so the column the detail table
/// describes always carries one.
fn composition_shows_label(column: usize, last_column: usize, stride: usize) -> bool {
    (last_column - column).is_multiple_of(stride)
}

/// The wording the dimension picker shows, so the caption cannot disagree with it.
fn dimension_label(dimension: &str) -> &str {
    match dimension {
        "task" => "data source",
        "filter_reason" => "filter reason",
        other => other,
    }
}

fn composition_chart_caption(composition: &RlCompositionResponse, dimension: &str) -> String {
    let label = dimension_label(dimension);
    let total = composition.steps.len();
    if total > MAX_COMPOSITION_COLUMNS {
        // "loaded" because `total` is the reach of the sample limit, not the run
        // length, which is usually far longer.
        format!(
            "Samples per step, by {label} · last {MAX_COMPOSITION_COLUMNS} of {total} loaded steps"
        )
    } else {
        format!("Samples per step, by {label}")
    }
}

/// Legend order comes from the aggregate buckets so colors stay stable as the
/// per-step ordering churns.
fn composition_legend(composition: &RlCompositionResponse) -> Vec<String> {
    composition
        .buckets
        .iter()
        .map(|bucket| bucket.key.clone())
        .collect()
}

fn nth_from_last<T: Clone>(items: &[T], offset: usize) -> Option<T> {
    items
        .len()
        .checked_sub(offset + 1)
        .map(|index| items[index].clone())
}

fn composition_color(legend: &[String], key: &str) -> &'static str {
    let index = legend.iter().position(|item| item == key).unwrap_or(0);
    COMPOSITION_COLORS[index % COMPOSITION_COLORS.len()]
}

/// Change in share against the previous step, in percentage points.
fn share_delta(previous: Option<&RlCompositionStep>, key: &str, share: f64) -> String {
    let Some(previous) = previous else {
        return "—".to_string();
    };
    let before = previous
        .buckets
        .iter()
        .find(|bucket| bucket.key == key)
        .map(|bucket| bucket.share)
        .unwrap_or(0.0);
    let delta = 100.0 * (share - before);
    if !delta.is_finite() || delta.abs() < 0.05 {
        return "0.0 pt".to_string();
    }
    let arrow = if delta > 0.0 { "▲" } else { "▼" };
    format!("{arrow}{:.1} pt", delta.abs())
}

#[component]
/// Nine-slice distribution of per-prompt pass rates. The two ends are coloured
/// apart because a prompt that every rollout failed, or every rollout passed,
/// teaches the policy nothing and is what dynamic sampling drops.
#[component]
fn PassHistogramBars(buckets: Vec<RlPassBucket>) -> Element {
    let max_prompts = buckets
        .iter()
        .map(|bucket| bucket.prompts)
        .max()
        .unwrap_or(1)
        .max(1);
    let last = buckets.len().saturating_sub(1);
    rsx! {
        div { class: "space-y-3 px-4 py-4",
            div { class: "flex items-end gap-2 h-32",
                for (index, bucket) in buckets.iter().enumerate() {
                    {
                        let height = format!(
                            "{:.0}%",
                            100.0 * bucket.prompts as f64 / max_prompts as f64
                        );
                        let share = format!("{:.1}%", bucket.share * 100.0);
                        let fill = if index == 0 {
                            "bg-red-400"
                        } else if index == last {
                            "bg-emerald-500"
                        } else {
                            "bg-blue-500"
                        };
                        rsx! {
                            div { class: "flex h-full flex-1 flex-col items-center justify-end gap-1",
                                span { class: "text-xs text-gray-500", "{bucket.prompts}" }
                                div {
                                    class: "w-full rounded-t {fill}",
                                    style: "height: {height}; min-height: 2px;",
                                    title: "pass rate {bucket.label}: {bucket.prompts} prompts ({share})",
                                }
                                span { class: "text-[11px] font-medium text-gray-700", "{bucket.label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn StalenessBars(buckets: Vec<RlStalenessBucket>) -> Element {
    let max_count = buckets
        .iter()
        .map(|bucket| bucket.count)
        .max()
        .unwrap_or(1)
        .max(1);
    rsx! {
        div { class: "space-y-3 px-4 py-4",
            div { class: "flex items-end gap-3 h-28",
                for bucket in buckets.iter() {
                    {
                        let height = format!(
                            "{:.0}%",
                            100.0 * bucket.count as f64 / max_count as f64
                        );
                        let share = format!("{:.0}%", bucket.share * 100.0);
                        rsx! {
                            div { class: "flex h-full flex-1 flex-col items-center justify-end gap-1",
                                span { class: "text-xs text-gray-500", "{bucket.count}" }
                                div {
                                    class: "w-full rounded-t bg-blue-500",
                                    style: "height: {height}; min-height: 2px;",
                                    title: "{bucket.label}: {bucket.count} ({share}), tokens={bucket.tokens}",
                                }
                                span { class: "text-[11px] font-medium text-gray-700", "{bucket.label}" }
                            }
                        }
                    }
                }
            }
            div { class: "grid grid-cols-2 gap-2 sm:grid-cols-5",
                for bucket in buckets.iter() {
                    {
                        let share = format!("{:.1}%", bucket.share * 100.0);
                        rsx! {
                            div { class: "rounded-md border border-gray-200 px-3 py-2 text-xs",
                                div { class: "font-medium text-gray-800", "staleness {bucket.label}" }
                                div { class: "text-gray-600", "{bucket.count} · {share}" }
                                div { class: "text-gray-500", "{bucket.tokens} tokens" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn chart_series(
    response: &RlSeriesResponse,
    name: &str,
    color: &'static str,
    x_axis: ChartXAxis,
) -> Vec<ChartSeries> {
    let Some(points) = response.series.get(name) else {
        return Vec::new();
    };
    vec![ChartSeries {
        label: name.to_string(),
        points: points
            .iter()
            .map(|point| ChartPoint {
                x: match x_axis {
                    ChartXAxis::Step => point.step as f64,
                    ChartXAxis::Time => {
                        if point.timestamp_ns > 0 {
                            point.timestamp_ns as f64
                        } else {
                            point.wall_time_s
                        }
                    }
                },
                y: point.value,
                step: point.step,
                // The spread, not min/max: a bucket's extremes widen as buckets get
                // coarser until the band fills the plot, whereas this tracks how much
                // the metric really moves. The extremes still reach the caption. Kept
                // at zero spread so a flat metric still reads as a summary.
                band: point
                    .spread
                    .filter(|spread| spread.is_finite())
                    .map(|spread| (point.value - spread, point.value + spread)),
            })
            .collect(),
        color,
        readout: latest_value(response, name),
        delta: step_delta(response, name).unwrap_or_default(),
    }]
}

/// Benchmark charts as `(title, harness caption, series)`.
/// One benchmark's latest result under a single run.
#[derive(Clone, Debug, Default, PartialEq)]
struct BenchmarkCell {
    score: Option<f64>,
    step: i64,
    /// Change against this run's previous evaluation of the same benchmark.
    delta: String,
}

/// A benchmark row across the runs being compared.
#[derive(Clone, Debug, PartialEq)]
struct BenchmarkMatrixRow {
    name: String,
    /// Harness and aggregation, which describe how the score was produced and so
    /// whether two runs' numbers are comparable at all.
    caption: String,
    cells: Vec<BenchmarkCell>,
}

/// Pivot per-run benchmark points into one row per benchmark, so two runs of the
/// same model family can be read side by side.
fn benchmark_matrix(runs: &[(String, RlBenchmarksResponse)]) -> Vec<BenchmarkMatrixRow> {
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    for (_, response) in runs {
        for point in &response.points {
            names.insert(point.name.clone(), harness_caption(point));
        }
    }
    names
        .into_iter()
        .map(|(name, caption)| {
            let cells = runs
                .iter()
                .map(|(_, response)| {
                    // Points arrive newest-last per benchmark.
                    let mut scores = response
                        .points
                        .iter()
                        .filter(|point| point.name == name)
                        .collect::<Vec<_>>();
                    scores.sort_by_key(|point| point.step);
                    let latest = scores.last();
                    BenchmarkCell {
                        score: latest.map(|point| point.score),
                        step: latest.map(|point| point.step).unwrap_or(-1),
                        delta: if scores.len() >= 2 {
                            format_metric_delta(
                                "benchmark.score",
                                scores[scores.len() - 1].score - scores[scores.len() - 2].score,
                            )
                        } else {
                            String::new()
                        },
                    }
                })
                .collect();
            BenchmarkMatrixRow {
                name,
                caption,
                cells,
            }
        })
        .collect()
}

/// Gap between the first two runs in a matrix row, in percentage points of score.
/// Empty unless both runs actually evaluated the benchmark.
fn benchmark_gap(row: &BenchmarkMatrixRow) -> String {
    match (
        row.cells.first().and_then(|cell| cell.score),
        row.cells.get(1).and_then(|cell| cell.score),
    ) {
        (Some(left), Some(right)) => format_metric_delta("benchmark.score", right - left),
        _ => String::new(),
    }
}

fn benchmark_series(response: &RlBenchmarksResponse) -> Vec<(String, String, ChartSeries)> {
    let mut grouped: BTreeMap<String, Vec<ChartPoint>> = BTreeMap::new();
    // The harness describing a benchmark can only change by re-running it, so
    // the newest point's metadata describes the whole line.
    let mut captions: BTreeMap<String, String> = BTreeMap::new();
    for point in &response.points {
        grouped
            .entry(point.name.clone())
            .or_default()
            .push(ChartPoint {
                x: point.step as f64,
                y: point.score,
                step: point.step,
                ..Default::default()
            });
        captions.insert(point.name.clone(), harness_caption(point));
    }
    grouped
        .into_iter()
        .enumerate()
        .map(|(index, (name, points))| {
            let caption = captions.get(&name).cloned().unwrap_or_default();
            let readout = points
                .last()
                .map(|point| format_metric_value("benchmark.score", point.y))
                .unwrap_or_default();
            let delta = if points.len() >= 2 {
                format_metric_delta(
                    "benchmark.score",
                    points[points.len() - 1].y - points[points.len() - 2].y,
                )
            } else {
                String::new()
            };
            (
                name.clone(),
                caption,
                ChartSeries {
                    label: name,
                    points,
                    color: COLORS[index % COLORS.len()],
                    readout,
                    delta,
                },
            )
        })
        .collect()
}

/// `mini-swe-agent · avg@3 · v1.1 · n=500`, skipping whatever was not reported.
fn harness_caption(point: &RlBenchmarkPoint) -> String {
    let mut parts = Vec::new();
    for field in [&point.harness, &point.aggregation, &point.version] {
        if !field.trim().is_empty() {
            parts.push(field.trim().to_string());
        }
    }
    if point.sample_count > 0 {
        parts.push(format!("n={}", point.sample_count));
    }
    parts.join(" · ")
}

/// Caption for the trend charts, naming the earliest step still on record when
/// the metric table has dropped the start of the run.
///
/// Each table is a fixed-size ring, so a long run loses its oldest metrics. A
/// chart that begins at step 2,550 would otherwise read as a run that started
/// there rather than one whose early history has aged out.
fn trends_subtitle(primary: &RunBroadcastData) -> String {
    const HINT: &str = "Click a point to open samples at that step.";
    let earliest = primary
        .trends
        .series
        .values()
        .filter_map(|points| points.first())
        .map(|point| point.step)
        .min();
    match earliest {
        // Only worth saying once the gap is more than a rounding artefact of the
        // bucket width; a run genuinely near its start needs no caveat.
        Some(step) if step > primary.trends.bucket_steps.max(1) * 2 => {
            format!(
                "{HINT} History starts at step {step}; earlier metrics have aged out of the table."
            )
        }
        _ => HINT.to_string(),
    }
}

fn series_stats(response: &RlSeriesResponse, name: &str) -> String {
    let Some(points) = response.series.get(name) else {
        return String::new();
    };
    if points.is_empty() {
        return String::new();
    }
    // A point may summarise a range of steps, in which case the run's true
    // extremes are in low/high; the values themselves are only bucket means.
    let min = points
        .iter()
        .map(|point| point.low.unwrap_or(point.value))
        .fold(f64::INFINITY, f64::min);
    let max = points
        .iter()
        .map(|point| point.high.unwrap_or(point.value))
        .fold(f64::NEG_INFINITY, f64::max);
    // Weight each point by the readings behind it, so a partly filled trailing
    // bucket does not count as much as a full one.
    let weight = |point: &probing_proto::prelude::RlMetricPoint| point.samples.max(1) as f64;
    let total = points.iter().map(weight).sum::<f64>();
    let mean = points
        .iter()
        .map(|point| point.value * weight(point))
        .sum::<f64>()
        / total;
    let mut caption = format!(
        "min {} · mean {} · max {}",
        format_metric_value(name, min),
        format_metric_value(name, mean),
        format_metric_value(name, max)
    );
    if response.bucket_steps > 1 {
        // Without this a reader would take each point for a single step.
        caption.push_str(&format!(" · {} steps/point", response.bucket_steps));
    }
    caption
}

fn latest_value(response: &RlSeriesResponse, name: &str) -> String {
    latest_numeric(response, name)
        .map(|value| format_metric_value(name, value))
        .unwrap_or_else(|| "—".to_string())
}

/// Change against the previous point of the same series, in the metric's unit.
fn step_delta(response: &RlSeriesResponse, name: &str) -> Option<String> {
    let points = response.series.get(name)?;
    if points.len() < 2 {
        return None;
    }
    let delta = points[points.len() - 1].value - points[points.len() - 2].value;
    let rendered = format_metric_delta(name, delta);
    (!rendered.is_empty()).then_some(rendered)
}

fn latest_numeric(response: &RlSeriesResponse, name: &str) -> Option<f64> {
    response
        .series
        .get(name)
        .and_then(|points| points.last())
        .map(|point| point.value)
}

fn delta_vs_first(response: &RlSeriesResponse, name: &str) -> String {
    let Some(points) = response.series.get(name) else {
        return String::new();
    };
    if points.len() < 2 {
        return String::new();
    }
    let first = points[0].value;
    let last = points[points.len() - 1].value;
    let delta = last - first;
    if !delta.is_finite() {
        return String::new();
    }
    format!(" ({:+.3})", delta)
}

fn latest_benchmarks(response: &RlBenchmarksResponse) -> Vec<(String, f64, i64)> {
    let mut latest: BTreeMap<String, (f64, i64)> = BTreeMap::new();
    for point in &response.points {
        latest
            .entry(point.name.clone())
            .and_modify(|(score, step)| {
                if point.step >= *step {
                    *score = point.score;
                    *step = point.step;
                }
            })
            .or_insert((point.score, point.step));
    }
    latest
        .into_iter()
        .map(|(name, (score, step))| (name, score, step))
        .collect()
}

fn benchmark_priority(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    if lower.contains("accuracy") {
        return 400;
    }
    if lower.contains("avg_pass") {
        return 300;
    }
    if lower.contains("online") {
        return 200;
    }
    100
}

fn pinned_benchmark(latest: &[(String, f64, i64)]) -> Option<(String, f64, i64)> {
    latest
        .iter()
        .max_by(|left, right| {
            benchmark_priority(&left.0)
                .cmp(&benchmark_priority(&right.0))
                .then_with(|| left.2.cmp(&right.2))
                .then_with(|| left.0.cmp(&right.0))
        })
        .cloned()
}

fn notice_level_class(level: &str) -> &'static str {
    match level {
        "warning" | "error" => "border-amber-200 bg-amber-50 text-amber-950",
        _ => "border-blue-200 bg-blue-50 text-blue-950",
    }
}

fn format_count(value: i64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.2}M", value as f64 / 1_000_000.0)
    } else if value >= 10_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn format_money(value: Option<f64>) -> String {
    match value {
        Some(amount) if amount.is_finite() => {
            if amount >= 1_000.0 {
                format!("${:.0}", amount)
            } else {
                format!("${:.2}", amount)
            }
        }
        _ => "—".to_string(),
    }
}

fn metric_label(name: &str) -> &'static str {
    OVERVIEW_METRICS
        .iter()
        .find_map(|(candidate, label, _)| (*candidate == name).then_some(*label))
        .unwrap_or("RL metric")
}

/// One-line explanation of what a canonical metric measures, for hover text.
/// Empty when the name is not one Probing publishes a definition for.
pub fn metric_description(name: &str) -> &'static str {
    OVERVIEW_METRICS
        .iter()
        .find_map(|(candidate, _, description)| (*candidate == name).then_some(*description))
        .unwrap_or("")
}

/// Hover text pairing the raw metric name with its definition.
pub fn metric_tooltip(name: &str) -> String {
    match metric_description(name) {
        "" => name.to_string(),
        description => format!("{name} — {description}"),
    }
}

fn format_metric(value: f64) -> String {
    if value.abs() >= 1_000.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

fn runtime_label(run: &RlRunSummary) -> String {
    if run.start_time_ns <= 0 {
        return "—".to_string();
    }
    let end = if run.end_time_ns > 0 {
        run.end_time_ns
    } else {
        run.timestamp_ns
    };
    format_duration((end - run.start_time_ns).max(0) / 1_000_000_000)
}

fn format_duration(seconds: i64) -> String {
    if seconds >= 3600 {
        format!("{}h {}m", seconds / 3600, seconds % 3600 / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

/// Render step position as `42/1000 (4.2%) · ETA 1h 12m` when the adapter
/// reported a total step count, and fall back to the bare step otherwise.
fn progress_label(response: &RlSeriesResponse, step: i64) -> String {
    let Some(total) = latest_numeric(response, "progress.total_steps")
        .filter(|total| total.is_finite() && *total >= 1.0)
    else {
        return format!("Step {step}");
    };
    let percent = latest_numeric(response, "progress.completed_ratio")
        .filter(|ratio| ratio.is_finite())
        .map(|ratio| format!(" ({:.1}%)", 100.0 * ratio))
        .unwrap_or_default();
    let eta = latest_numeric(response, "progress.eta_s")
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(|seconds| format!(" · ETA {}", format_duration(seconds as i64)))
        .unwrap_or_default();
    format!("Step {step}/{total:.0}{percent}{eta}")
}

fn nonempty(value: &str) -> String {
    if value.is_empty() {
        "unknown".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::RlMetricPoint;

    const HOUR_NS: i64 = 3_600_000_000_000;

    fn labelled_steps(first_step: i64, columns: usize) -> Vec<i64> {
        let last = columns - 1;
        let stride = composition_label_stride(columns, first_step + last as i64);
        (0..columns)
            .filter(|column| composition_shows_label(*column, last, stride))
            .map(|column| first_step + column as i64)
            .collect()
    }

    fn broadcast_with_first_step(step: i64, bucket_steps: i64) -> RunBroadcastData {
        RunBroadcastData {
            trends: RlSeriesResponse {
                series: BTreeMap::from([(
                    "reward.mean".to_string(),
                    vec![RlMetricPoint {
                        step,
                        ..Default::default()
                    }],
                )]),
                bucket_steps,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn trends_caption_admits_when_early_history_has_aged_out() {
        let caption = trends_subtitle(&broadcast_with_first_step(2550, 8));
        assert!(
            caption.contains("History starts at step 2550"),
            "got {caption}"
        );
        assert!(caption.contains("aged out"), "got {caption}");
    }

    #[test]
    fn trends_caption_stays_quiet_near_the_start_of_a_run() {
        // First bucket of an eight-step width: nothing has been dropped, so a
        // caveat here would be misleading.
        assert_eq!(
            trends_subtitle(&broadcast_with_first_step(8, 8)),
            "Click a point to open samples at that step."
        );
        assert_eq!(
            trends_subtitle(&broadcast_with_first_step(1, 0)),
            "Click a point to open samples at that step."
        );
    }

    #[test]
    fn stats_of_a_bucketed_series_report_the_true_range() {
        // Two buckets of six steps each, as the server sends them. Reading the
        // extremes off the bucket means would understate the run's range.
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: BTreeMap::from([(
                "reward.mean".to_string(),
                vec![
                    RlMetricPoint {
                        step: 6,
                        value: 0.25,
                        low: Some(0.05),
                        high: Some(0.45),
                        samples: 6,
                        ..Default::default()
                    },
                    RlMetricPoint {
                        step: 12,
                        value: 0.75,
                        low: Some(0.55),
                        high: Some(0.95),
                        samples: 2,
                        ..Default::default()
                    },
                ],
            )]),
            bucket_steps: 6,
        };
        // Extremes come from low/high, and the mean is weighted by samples:
        // (0.25*6 + 0.75*2) / 8 = 0.375, not the 0.5 an unweighted average of the
        // two bucket means would give. The step width tells the reader a point is
        // not a single step.
        assert_eq!(
            series_stats(&response, "reward.mean"),
            "min 0.0500 · mean 0.3750 · max 0.9500 · 6 steps/point"
        );
    }

    #[test]
    fn the_drawn_band_is_the_spread_not_the_extremes() {
        // A bucket whose extremes are far wider than its spread, which is the usual
        // shape for a noisy metric. Drawing min/max fills the plot with colour and
        // hides the line, so only the spread may reach the chart.
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: BTreeMap::from([(
                "time.step_s".to_string(),
                vec![RlMetricPoint {
                    step: 8,
                    value: 14.0,
                    low: Some(12.0),
                    high: Some(16.0),
                    spread: Some(0.4),
                    samples: 8,
                    ..Default::default()
                }],
            )]),
            bucket_steps: 8,
        };
        let series = chart_series(&response, "time.step_s", "#000", ChartXAxis::Step);
        assert_eq!(series[0].points[0].band, Some((13.6, 14.4)));
    }

    #[test]
    fn a_series_without_a_spread_is_drawn_as_raw_points() {
        // What a server that does not aggregate sends. Inventing a band from the
        // value alone would claim a range the reading does not have.
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: BTreeMap::from([(
                "time.step_s".to_string(),
                vec![RlMetricPoint {
                    step: 1,
                    value: 14.0,
                    ..Default::default()
                }],
            )]),
            ..Default::default()
        };
        let series = chart_series(&response, "time.step_s", "#000", ChartXAxis::Step);
        assert_eq!(series[0].points[0].band, None);
    }

    #[test]
    fn stats_of_a_raw_series_are_unchanged() {
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: BTreeMap::from([(
                "reward.mean".to_string(),
                vec![
                    RlMetricPoint {
                        step: 1,
                        value: 0.2,
                        ..Default::default()
                    },
                    RlMetricPoint {
                        step: 2,
                        value: 0.4,
                        ..Default::default()
                    },
                ],
            )]),
            ..Default::default()
        };
        let caption = series_stats(&response, "reward.mean");
        assert_eq!(caption, "min 0.2000 · mean 0.3000 · max 0.4000");
    }

    #[test]
    fn composition_axis_always_labels_the_newest_step() {
        // 48 columns ending at step 90: counting forward from zero stopped at 88
        // and left the step the detail table describes unlabelled.
        let labels = labelled_steps(43, 48);
        assert_eq!(labels.last(), Some(&90));
        assert_eq!(labels.len(), 16);
        assert_eq!(&labels[..3], &[45, 48, 51]);
    }

    #[test]
    fn composition_axis_labels_every_column_when_run_is_short() {
        assert_eq!(labelled_steps(1, 9), (1..=9).collect::<Vec<_>>());
        assert_eq!(composition_label_stride(1, 1), 1);
    }

    #[test]
    fn composition_axis_thins_out_for_wider_step_numbers() {
        // Same column count, but five-digit steps get fewer labels than two-digit.
        assert_eq!(composition_label_stride(48, 90), 3);
        assert_eq!(composition_label_stride(48, 2079), 4);
        assert_eq!(composition_label_stride(48, 20790), 5);
        assert_eq!(labelled_steps(2032, 48).last(), Some(&2079));
    }

    #[test]
    fn composition_caption_matches_the_dimension_picker_wording() {
        let composition = RlCompositionResponse {
            steps: vec![RlCompositionStep::default(); 3],
            ..Default::default()
        };
        assert_eq!(
            composition_chart_caption(&composition, "task"),
            "Samples per step, by data source"
        );
        assert_eq!(dimension_label("filter_reason"), "filter reason");
        assert_eq!(dimension_label("category"), "category");

        let long = RlCompositionResponse {
            steps: vec![RlCompositionStep::default(); 90],
            ..Default::default()
        };
        assert_eq!(
            composition_chart_caption(&long, "category"),
            "Samples per step, by category · last 48 of 90 loaded steps"
        );
    }

    fn benchmark_point(name: &str, step: i64, score: f64) -> RlBenchmarkPoint {
        RlBenchmarkPoint {
            name: name.to_string(),
            step,
            score,
            sample_count: 10,
            version: "v1".into(),
            timestamp_ns: step,
            harness: "harness-a".into(),
            aggregation: "avg@3".into(),
        }
    }

    #[test]
    fn benchmark_matrix_pivots_runs_into_columns() {
        let runs = vec![
            (
                "run-pro".to_string(),
                RlBenchmarksResponse {
                    run_id: "run-pro".into(),
                    points: vec![
                        benchmark_point("code@live", 10, 0.40),
                        benchmark_point("code@live", 20, 0.50),
                        benchmark_point("math@hard", 20, 0.70),
                    ],
                },
            ),
            (
                "run-flash".to_string(),
                RlBenchmarksResponse {
                    run_id: "run-flash".into(),
                    points: vec![benchmark_point("code@live", 20, 0.55)],
                },
            ),
        ];
        let rows = benchmark_matrix(&runs);
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            ["code@live", "math@hard"]
        );

        let code = &rows[0];
        assert_eq!(code.cells[0].score, Some(0.50));
        assert_eq!(code.cells[0].step, 20);
        // Delta is against this run's own previous evaluation.
        assert!(!code.cells[0].delta.is_empty());
        assert_eq!(code.cells[1].score, Some(0.55));
        // The second run only ran it once, so it has no step-over-step delta.
        assert!(code.cells[1].delta.is_empty());
        assert!(!benchmark_gap(code).is_empty());

        // A benchmark only one run evaluated still gets a row, with a blank cell
        // and no gap rather than a fabricated zero.
        let math = &rows[1];
        assert_eq!(math.cells[0].score, Some(0.70));
        assert_eq!(math.cells[1].score, None);
        assert_eq!(math.cells[1].step, -1);
        assert_eq!(benchmark_gap(math), "");
    }

    #[test]
    fn benchmark_matrix_uses_the_newest_step_regardless_of_input_order() {
        let runs = vec![(
            "run".to_string(),
            RlBenchmarksResponse {
                run_id: "run".into(),
                points: vec![
                    benchmark_point("code@live", 30, 0.60),
                    benchmark_point("code@live", 10, 0.20),
                ],
            },
        )];
        let rows = benchmark_matrix(&runs);
        assert_eq!(rows[0].cells[0].score, Some(0.60));
        assert_eq!(rows[0].cells[0].step, 30);
        // Single-run matrices have nothing to compare against.
        assert_eq!(benchmark_gap(&rows[0]), "");
    }

    #[test]
    fn pass_rate_of_an_unjudged_source_is_not_shown_as_zero() {
        assert_eq!(pass_rate_label(None), "—");
        assert_eq!(pass_rate_label(Some(0.0)), "0.0 %");
        assert_eq!(pass_rate_label(Some(0.3333)), "33.3 %");
        assert_eq!(pass_rate_label(Some(1.0)), "100.0 %");
    }

    #[test]
    fn metric_tooltip_pairs_the_name_with_its_definition() {
        assert_eq!(
            metric_tooltip("reward.mean"),
            "reward.mean — Mean reward over trajectories trained in one step"
        );
        // Names the dashboard has no definition for fall back to just the name.
        assert_eq!(metric_tooltip("custom.thing"), "custom.thing");
        assert_eq!(metric_description("custom.thing"), "");
    }

    #[test]
    fn every_headline_metric_is_a_known_gauge() {
        // Charts colour themselves by position and title themselves through
        // OVERVIEW_METRICS, so a headline name missing from that list renders as
        // an untitled chart, and a list longer than COLORS used to panic.
        for name in HEADLINE_METRICS {
            assert!(
                OVERVIEW_METRICS.iter().any(|(gauge, _, _)| *gauge == name),
                "{name} is charted but absent from OVERVIEW_METRICS"
            );
            assert_ne!(metric_label(name), "RL metric", "{name} has no label");
        }
    }

    #[test]
    fn age_label_counts_back_from_newest_telemetry() {
        let now = 1_000 * HOUR_NS;
        assert_eq!(age_label(now, now - 31 * HOUR_NS), "1d 07h");
        assert_eq!(age_label(now, now - 2 * HOUR_NS), "2h 00m");
        assert_eq!(age_label(now, now - 12 * 60_000_000_000), "12m");
        assert_eq!(age_label(now, now), "just now");
        // A notice stamped ahead of the newest telemetry reads as current, not negative.
        assert_eq!(age_label(now, now + HOUR_NS), "just now");
        assert_eq!(age_label(0, now), "");
    }

    #[test]
    fn batch_shape_reports_group_size_when_known() {
        let mut series = BTreeMap::from([(
            "sampler.batch_size".to_string(),
            vec![RlMetricPoint {
                step: 1,
                wall_time_s: 0.0,
                timestamp_ns: 1,
                value: 1_568.0,
                ..Default::default()
            }],
        )]);
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: series.clone(),
            ..Default::default()
        };
        assert_eq!(batch_shape_label(&response), "1568");

        series.insert(
            "sampler.group_size".to_string(),
            vec![RlMetricPoint {
                step: 1,
                wall_time_s: 0.0,
                timestamp_ns: 1,
                value: 16.0,
                ..Default::default()
            }],
        );
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series,
            ..Default::default()
        };
        assert_eq!(batch_shape_label(&response), "1568 × 16 seqs");
    }

    #[test]
    fn harness_caption_skips_unreported_fields() {
        let point = RlBenchmarkPoint {
            name: "DeepSWE".into(),
            step: 30,
            score: 72.57,
            sample_count: 500,
            version: "v1.1".into(),
            timestamp_ns: 1,
            harness: "mini-swe-agent".into(),
            aggregation: "avg@3".into(),
        };
        assert_eq!(
            harness_caption(&point),
            "mini-swe-agent · avg@3 · v1.1 · n=500"
        );

        let bare = RlBenchmarkPoint {
            harness: String::new(),
            aggregation: String::new(),
            version: String::new(),
            sample_count: 0,
            ..point
        };
        assert_eq!(harness_caption(&bare), "");
    }

    #[test]
    fn offset_label_spans_days_and_hours() {
        let start = 1_000 * HOUR_NS;
        // Inside the first hour a day counter would hide all the detail.
        assert_eq!(offset_label(start, start), "T+00:00");
        assert_eq!(offset_label(start, start + 252_000_000_000), "T+04:12");
        assert_eq!(offset_label(start, start + 26 * HOUR_NS), "T+1d 02:00");
        // Unknown start or a timestamp before it gets no label rather than a wrong one.
        assert_eq!(offset_label(0, start), "");
        assert_eq!(offset_label(start, start - HOUR_NS), "");
    }

    #[test]
    fn timeline_interleaves_steps_and_events_newest_first() {
        let start = 100 * HOUR_NS;
        let data = RunBroadcastData {
            run: RlRunSummary {
                run_id: "run".into(),
                start_time_ns: start,
                global_step: 2,
                ..Default::default()
            },
            sampler: None,
            series: RlSeriesResponse::default(),
            trends: RlSeriesResponse {
                run_id: "run".into(),
                series: BTreeMap::from([
                    (
                        "sampler.avg_pass".into(),
                        vec![
                            RlMetricPoint {
                                step: 1,
                                wall_time_s: 0.0,
                                timestamp_ns: start + HOUR_NS,
                                value: 0.50,
                                ..Default::default()
                            },
                            RlMetricPoint {
                                step: 2,
                                wall_time_s: 0.0,
                                timestamp_ns: start + 3 * HOUR_NS,
                                value: 0.55,
                                ..Default::default()
                            },
                        ],
                    ),
                    (
                        "train.tokens".into(),
                        vec![RlMetricPoint {
                            step: 2,
                            wall_time_s: 0.0,
                            timestamp_ns: start + 3 * HOUR_NS,
                            value: 3.43e9,
                            ..Default::default()
                        }],
                    ),
                ]),
                ..Default::default()
            },
            benchmarks: RlBenchmarksResponse::default(),
            events: vec![RlEvent {
                timestamp_ns: start + 2 * HOUR_NS,
                step: 1,
                level: "warning".into(),
                kind: "restart".into(),
                message: "trainer restarted".into(),
            }],
        };

        let rows = timeline_rows(&data, TIMELINE_LIMIT);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].offset, "T+0d 03:00");
        assert_eq!(rows[2].offset, "T+0d 01:00");
        assert!(rows[0]
            .headline
            .starts_with("step 2 · sampler.avg_pass 0.5500 ▲0.0500"));
        assert_eq!(rows[0].trailing, "3.43B tok");
        assert_eq!(rows[1].headline, "trainer restarted");
        assert_eq!(rows[1].level, "warning");
        assert!(rows[2].headline.starts_with("step 1"));
        // The first step has no predecessor, so it reports no delta.
        assert!(!rows[2].headline.contains('▲'));
    }

    #[test]
    fn delta_vs_first_formats_gain() {
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: BTreeMap::from([(
                "sampler.avg_pass".into(),
                vec![
                    RlMetricPoint {
                        step: 1,
                        wall_time_s: 1.0,
                        timestamp_ns: 1,
                        value: 0.50,
                        ..Default::default()
                    },
                    RlMetricPoint {
                        step: 3,
                        wall_time_s: 3.0,
                        timestamp_ns: 3,
                        value: 0.61,
                        ..Default::default()
                    },
                ],
            )]),
            ..Default::default()
        };
        assert_eq!(delta_vs_first(&response, "sampler.avg_pass"), " (+0.110)");
    }

    fn gauge(name: &str, value: f64) -> (String, Vec<RlMetricPoint>) {
        (
            name.into(),
            vec![RlMetricPoint {
                step: 1,
                wall_time_s: 1.0,
                timestamp_ns: 1,
                value,
                ..Default::default()
            }],
        )
    }

    #[test]
    fn progress_label_reports_share_and_eta() {
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: BTreeMap::from([
                gauge("progress.total_steps", 10_000.0),
                gauge("progress.completed_ratio", 0.0041),
                gauge("progress.eta_s", 142_620.0),
            ]),
            ..Default::default()
        };
        assert_eq!(
            progress_label(&response, 41),
            "Step 41/10000 (0.4%) · ETA 39h 37m"
        );
    }

    #[test]
    fn progress_label_falls_back_without_total_steps() {
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: BTreeMap::from([gauge("progress.eta_s", 900.0)]),
            ..Default::default()
        };
        assert_eq!(progress_label(&response, 41), "Step 41");
    }

    #[test]
    fn progress_label_omits_eta_once_finished() {
        let response = RlSeriesResponse {
            run_id: "run".into(),
            series: BTreeMap::from([
                gauge("progress.total_steps", 100.0),
                gauge("progress.completed_ratio", 1.0),
            ]),
            ..Default::default()
        };
        assert_eq!(progress_label(&response, 100), "Step 100/100 (100.0%)");
    }

    #[test]
    fn pinned_benchmark_prefers_accuracy_then_avg_pass() {
        let latest = vec![
            ("code@live".into(), 0.9, 20),
            ("math@accuracy".into(), 0.4, 10),
            ("agent@online/avg_pass".into(), 0.7, 30),
        ];
        assert_eq!(
            pinned_benchmark(&latest).map(|item| item.0),
            Some("math@accuracy".into())
        );
        let without_accuracy = vec![
            ("code@live".into(), 0.9, 20),
            ("agent@online/avg_pass".into(), 0.7, 30),
        ];
        assert_eq!(
            pinned_benchmark(&without_accuracy).map(|item| item.0),
            Some("agent@online/avg_pass".into())
        );
    }
}

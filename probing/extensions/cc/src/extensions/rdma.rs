use probing_core::core::EngineError;
use probing_core::core::Maybe;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionOption;
use probing_core::sync::lock_mutex;

use datafusion::arrow::array::{GenericStringBuilder, RecordBatch};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};

use probing_core::core::{CustomTable, ProbeExtensionCall, TableProbeDataSource};

use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use async_trait::async_trait;
use std::collections::HashMap;

use std::fs::File;
use std::io::{self, Read};
use std::time::{Duration, Instant};

static GLOBAL_HCA_NAME: OnceLock<Mutex<String>> = OnceLock::new();
static GLOBAL_HCA_SAMPLE_RATE: OnceLock<Mutex<f64>> = OnceLock::new();

fn lock_hca_name() -> std::sync::MutexGuard<'static, String> {
    lock_mutex(
        GLOBAL_HCA_NAME.get_or_init(|| Mutex::new(String::new())),
        "rdma HCA name",
    )
}

fn lock_sample_rate() -> std::sync::MutexGuard<'static, f64> {
    lock_mutex(
        GLOBAL_HCA_SAMPLE_RATE.get_or_init(|| Mutex::new(0.0)),
        "rdma sample rate",
    )
}

#[derive(Default, Debug)]
pub struct RdmaTable {}

impl CustomTable for RdmaTable {
    fn name() -> &'static str {
        "mlx_hca"
    }

    fn schema() -> datafusion::arrow::datatypes::SchemaRef {
        SchemaRef::new(Schema::new(vec![
            Field::new("hca_name", DataType::Utf8, false),
            Field::new("port_rcv_packets", DataType::Float64, false),
            Field::new("port_rcv_data", DataType::Float64, false),
            Field::new("port_xmit_packets", DataType::Float64, false),
            Field::new("port_xmit_data", DataType::Float64, false),
            Field::new("link_downed", DataType::Float64, false),
            Field::new("np_cnp_sent", DataType::Float64, false),
            Field::new("np_ecn_marked_roce_packets", DataType::Float64, false),
            Field::new("rcv_pkts_rate", DataType::Float64, false),
            Field::new("snd_pkts_rate", DataType::Float64, false),
        ]))
    }

    fn data() -> Vec<datafusion::arrow::array::RecordBatch> {
        let hca_name = lock_hca_name().clone();

        let mut monitor = RDMAMonitor::new(&hca_name);
        monitor.obtain_newset();

        let mut hca_name = GenericStringBuilder::<i32>::new();
        let mut port_rcv_packets = datafusion::arrow::array::Float64Builder::new();
        let mut port_rcv_data = datafusion::arrow::array::Float64Builder::new();
        let mut port_xmit_packets = datafusion::arrow::array::Float64Builder::new();
        let mut port_xmit_data = datafusion::arrow::array::Float64Builder::new();
        let mut link_downed = datafusion::arrow::array::Float64Builder::new();
        let mut np_cnp_sent = datafusion::arrow::array::Float64Builder::new();
        let mut np_ecn_marked_roce_packets = datafusion::arrow::array::Float64Builder::new();
        let mut rcv_pkts_rate = datafusion::arrow::array::Float64Builder::new();
        let mut snd_pkts_rate = datafusion::arrow::array::Float64Builder::new();
        hca_name.append_value(monitor.hca_name.clone());
        port_rcv_packets.append_value(monitor.read_counter("port_rcv_packets"));
        port_rcv_data.append_value(monitor.read_counter("port_rcv_data"));
        port_xmit_packets.append_value(monitor.read_counter("port_xmit_packets"));
        port_xmit_data.append_value(monitor.read_counter("port_xmit_data"));
        link_downed.append_value(monitor.read_counter("link_downed"));
        np_cnp_sent.append_value(monitor.read_counter("np_cnp_sent"));
        np_ecn_marked_roce_packets.append_value(monitor.read_counter("np_ecn_marked_roce_packets"));

        let sleep_time = *lock_sample_rate() as u64;

        thread::sleep(Duration::from_secs(sleep_time));

        rcv_pkts_rate.append_value(monitor.calculate_rate(
            Some(monitor.read_counter("port_rcv_packets")),
            monitor.previous_port_rcv_packets,
            monitor.last_measurement_time.map(|t| t.elapsed()),
        ));
        snd_pkts_rate.append_value(monitor.calculate_rate(
            Some(monitor.read_counter("port_xmit_packets")),
            monitor.previous_port_xmit_packets,
            monitor.last_measurement_time.map(|t| t.elapsed()),
        ));
        let rbs = RecordBatch::try_new(
            Self::schema(),
            vec![
                Arc::new(hca_name.finish()),
                Arc::new(port_rcv_packets.finish()),
                Arc::new(port_rcv_data.finish()),
                Arc::new(port_xmit_packets.finish()),
                Arc::new(port_xmit_data.finish()),
                Arc::new(link_downed.finish()),
                Arc::new(np_cnp_sent.finish()),
                Arc::new(np_ecn_marked_roce_packets.finish()),
                Arc::new(rcv_pkts_rate.finish()),
                Arc::new(snd_pkts_rate.finish()),
            ],
        );
        if let Ok(rbs) = rbs {
            vec![rbs]
        } else {
            Default::default()
        }
    }
}

pub type RdmaProbeDataSource = TableProbeDataSource<RdmaTable>;

#[derive(Debug, Default, ProbeExtension)]
pub struct RdmaProbeExtension {
    #[option(aliases=["sample.rate"])]
    sample_rate: Maybe<f64>,

    #[option(aliases=["hca.name"])]
    hca_name: Maybe<String>,
}

#[async_trait]
impl ProbeExtensionCall for RdmaProbeExtension {
    async fn call(
        &self,
        path: &str,
        _params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Vec<u8>, EngineError> {
        if path.is_empty() {
            let hca_name = resolve_hca_name(body)?;
            return Ok(format_rdma_snapshot(&hca_name)?.into_bytes());
        }
        Err(EngineError::UnsupportedCall)
    }
}

fn resolve_hca_name(body: &[u8]) -> Result<String, EngineError> {
    if !body.is_empty() {
        let name = String::from_utf8_lossy(body).trim().to_string();
        if !name.is_empty() {
            *lock_hca_name() = name.clone();
            return Ok(name);
        }
    }

    let name = lock_hca_name().clone();
    if name.is_empty() {
        return Err(EngineError::InvalidOptionValue(
            RdmaProbeExtension::OPTION_HCA_NAME.to_string(),
            "HCA name required (POST body or SET rdma.hca.name)".to_string(),
        ));
    }
    Ok(name)
}

fn format_rdma_snapshot(hca_name: &str) -> Result<String, EngineError> {
    let mut monitor = RDMAMonitor::new(hca_name);
    monitor.obtain_newset();

    let sleep_secs = lock_sample_rate().max(0.0) as u64;
    if sleep_secs > 0 {
        thread::sleep(Duration::from_secs(sleep_secs));
    }

    let port_rcv_packets = monitor.read_counter("port_rcv_packets");
    let port_rcv_data = monitor.read_counter("port_rcv_data");
    let port_xmit_packets = monitor.read_counter("port_xmit_packets");
    let port_xmit_data = monitor.read_counter("port_xmit_data");
    let link_downed = monitor.read_counter("link_downed");
    let np_cnp_sent = monitor.read_counter("np_cnp_sent");
    let np_ecn_marked = monitor.read_counter("np_ecn_marked_roce_packets");
    let rcv_pkts_rate = monitor.calculate_rate(
        Some(port_rcv_packets),
        monitor.previous_port_rcv_packets,
        monitor.last_measurement_time.map(|t| t.elapsed()),
    );
    let snd_pkts_rate = monitor.calculate_rate(
        Some(port_xmit_packets),
        monitor.previous_port_xmit_packets,
        monitor.last_measurement_time.map(|t| t.elapsed()),
    );

    Ok(format!(
        "hca_name: {hca_name}\n\
         port_rcv_packets: {port_rcv_packets}\n\
         port_rcv_data: {port_rcv_data}\n\
         port_xmit_packets: {port_xmit_packets}\n\
         port_xmit_data: {port_xmit_data}\n\
         link_downed: {link_downed}\n\
         np_cnp_sent: {np_cnp_sent}\n\
         np_ecn_marked_roce_packets: {np_ecn_marked}\n\
         rcv_pkts_rate: {rcv_pkts_rate:.2}\n\
         snd_pkts_rate: {snd_pkts_rate:.2}\n"
    ))
}

impl RdmaProbeExtension {
    fn set_sample_rate(&mut self, sample_rate: Maybe<f64>) -> Result<(), EngineError> {
        if let Maybe::Just(rate) = sample_rate {
            if !(0.0..=20.0).contains(&rate) {
                return Err(EngineError::InvalidOptionValue(
                    Self::OPTION_SAMPLE_RATE.to_string(),
                    rate.to_string(),
                ));
            }

            *lock_sample_rate() = rate;
        }

        self.sample_rate = sample_rate;

        Ok(())
    }

    fn set_hca_name(&mut self, hca_name: Maybe<String>) -> Result<(), EngineError> {
        self.hca_name = hca_name;

        if let Maybe::Just(ref name) = self.hca_name {
            if name.is_empty() {
                return Err(EngineError::InvalidOptionValue(
                    Self::OPTION_HCA_NAME.to_string(),
                    "HCA name cannot be empty".to_string(),
                ));
            }

            *lock_hca_name() = name.clone();
        }

        Ok(())
    }
}

struct RDMAMonitor {
    hca_name: String,
    previous_port_rcv_packets: Option<f64>,
    previous_port_xmit_packets: Option<f64>,
    last_measurement_time: Option<Instant>,
}

impl RDMAMonitor {
    fn new(hca_name: &str) -> Self {
        RDMAMonitor {
            hca_name: hca_name.to_string(),
            previous_port_rcv_packets: None,
            previous_port_xmit_packets: None,
            last_measurement_time: None,
        }
    }

    fn read_counter(&self, counter_name: &str) -> f64 {
        let path = if counter_name == "np_cnp_sent" || counter_name == "np_ecn_marked_roce_packets"
        {
            format!(
                "/sys/class/infiniband/{}/ports/1/hw_counters/{}",
                self.hca_name, counter_name
            )
        } else {
            format!(
                "/sys/class/infiniband/{}/ports/1/counters/{}",
                self.hca_name, counter_name
            )
        };

        read_file_to_f64(&path).unwrap_or(0.0)
    }

    fn calculate_rate(
        &self,
        current: Option<f64>,
        previous: Option<f64>,
        interval: Option<Duration>,
    ) -> f64 {
        let (Some(current), Some(previous), Some(interval)) = (current, previous, interval) else {
            return 0.0;
        };
        let interval = interval.as_secs_f64();

        let diff = if current < previous {
            current + 2u64.pow(64) as f64 - previous
        } else {
            current - previous
        };

        diff / interval
    }

    fn obtain_newset(&mut self) {
        let port_rcv_packets = self.read_counter("port_rcv_packets");
        let port_xmit_packets = self.read_counter("port_xmit_packets");
        self.previous_port_rcv_packets = Some(port_rcv_packets);
        self.previous_port_xmit_packets = Some(port_xmit_packets);
        self.last_measurement_time = Some(Instant::now());
    }
}

fn read_file_to_f64(path: &str) -> io::Result<f64> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    contents.trim().parse::<f64>().map_err(io::Error::other)
}

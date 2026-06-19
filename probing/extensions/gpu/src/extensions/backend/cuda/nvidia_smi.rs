use std::collections::HashMap;
use std::process::Command;

/// Per-device stats from `nvidia-smi` (multi-GPU / 8× datacenter nodes).
#[derive(Debug, Clone, Copy, Default)]
pub struct NvidiaDeviceStats {
    pub gpu_util_pct: f32,
    /// Memory controller utilization (%), distinct from VRAM fill ratio.
    pub mem_controller_util_pct: f32,
}

/// Batch-read utilization for all visible GPUs (one subprocess per sample tick).
pub fn read_utilization_by_index() -> HashMap<i32, NvidiaDeviceStats> {
    let output = match Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,utilization.gpu,utilization.memory",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return HashMap::new(),
    };

    let mut map = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split(',').map(str::trim);
        let Some(index_str) = parts.next() else {
            continue;
        };
        let Ok(index) = index_str.parse::<i32>() else {
            continue;
        };
        let Some(gpu_str) = parts.next() else {
            continue;
        };
        let Ok(gpu_util) = gpu_str.parse::<f32>() else {
            continue;
        };
        let mem_util = parts
            .next()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0);
        map.insert(
            index,
            NvidiaDeviceStats {
                gpu_util_pct: gpu_util,
                mem_controller_util_pct: mem_util,
            },
        );
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nvidia_smi_csv_line() {
        let mut map = HashMap::new();
        for line in ["0, 45, 12", "1, 80, 30"] {
            let mut parts = line.split(',').map(str::trim);
            let index: i32 = parts.next().unwrap().parse().unwrap();
            let gpu_util: f32 = parts.next().unwrap().parse().unwrap();
            let mem_util: f32 = parts.next().unwrap().parse().unwrap();
            map.insert(
                index,
                NvidiaDeviceStats {
                    gpu_util_pct: gpu_util,
                    mem_controller_util_pct: mem_util,
                },
            );
        }
        assert_eq!(map.get(&0).unwrap().gpu_util_pct, 45.0);
        assert_eq!(map.get(&1).unwrap().mem_controller_util_pct, 30.0);
    }
}

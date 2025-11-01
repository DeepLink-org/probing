use std::{collections::BTreeMap, thread};

use anyhow::Result;
use html_escape::encode_text;
use inferno;
use log::{error, warn};

use crate::extensions::python::PythonPlugin;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Frame {
    stage: String,
    module: String,
}

pub fn query_profiling() -> Result<Vec<String>> {
    let data = thread::spawn(|| {
        let engine = probing_core::create_engine()
            .with_plugin(PythonPlugin::create("python"))
            .build()?;

        let query = r#"
        select module, stage, median(duration)
            from python.torch_trace 
            where module <> 'None'
            group by module, stage
            order by (stage, module);
        "#;

        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { engine.async_query(query).await })
    })
    .join()
    .map_err(|_| anyhow::anyhow!("error joining thread"))??;

    let mut frames = BTreeMap::default();

    for line in data.iter() {
        let frame = Frame {
            stage: line[1].to_string(),
            module: line[0].to_string(),
        };
        let duration = match line[2] {
            probing_proto::types::Ele::F32(x) => x as f64,
            probing_proto::types::Ele::F64(x) => x,
            _ => 0 as f64,
        };

        frames
            .entry(frame.clone())
            .and_modify(|x| *x += duration)
            .or_insert(duration);

        let mut parts = frame.module.split(".").collect::<Vec<_>>();
        if parts.len() > 1 {
            parts.pop();
            let parent = Frame {
                stage: frame.stage.clone(),
                module: parts.join("."),
            };
            frames.entry(parent).and_modify(|x| *x -= duration);
        }
    }

    Ok(frames
        .iter()
        .map(|(frame, duration)| {
            let mut line = String::default();
            line.push_str(&frame.stage);
            line.push(';');

            let parts = frame.module.split(".").collect::<Vec<_>>();
            for part in parts {
                line.push_str(part);
                line.push(';');
            }

            let duration = if *duration < 0. { 0. } else { *duration };

            line.push_str(&format!(" {}", (duration * 100000.) as isize));

            line
        })
        .collect())
}

pub fn flamegraph() -> String {
    let mut graph: Vec<u8> = vec![];
    match query_profiling() {
        Err(err) => {
            error!("Failed to query torch profiling data: {err}");
            return empty_svg("Torch profiling data unavailable");
        }
        Ok(lines) => {
            if lines.is_empty() {
                warn!("Torch profiling returned no samples; skipping flamegraph generation");
                return empty_svg("No torch profiling samples collected");
            }

            let line_refs = lines.iter().map(|x| x.as_str()).collect::<Vec<_>>();
            println!("Generating torch flamegraph with:\n {:?}", line_refs);
            let mut opt = inferno::flamegraph::Options::default();
            opt.deterministic = true;
            match inferno::flamegraph::from_lines(&mut opt, line_refs, &mut graph) {
                Ok(_) => String::from_utf8(graph)
                    .unwrap_or_else(|_| empty_svg("Invalid flamegraph output")),
                Err(e) => {
                    error!("Failed to build torch flamegraph: {e}");
                    empty_svg("Unable to build torch flamegraph")
                }
            }
        }
    }
}

fn empty_svg(message: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='800' height='120'>\
         <rect width='100%' height='100%' fill='#f5f5f5'/>\
         <text x='50%' y='50%' dominant-baseline='middle' text-anchor='middle'\
           font-family='sans-serif' font-size='16' fill='#666'>{}</text>\
         </svg>",
        encode_text(message)
    )
}

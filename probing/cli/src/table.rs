#[cfg(unix)]
use nix::ioctl_read;
#[cfg(unix)]
use nix::libc;
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd};
use tabled::builder::Builder;
use tabled::grid::config::Position;
use tabled::grid::records::{
    vec_records::{Text, VecRecords},
    ExactRecords, Records,
};
use tabled::settings::{
    object::Segment,
    peaker::{PriorityMax, PriorityMin},
    Alignment, Settings, Style, Width,
};

use probing_proto::prelude::{DataFrame, Ele};

pub struct Table {
    data: VecRecords<Text<String>>,
}

impl Table {
    pub fn new(ncol: usize, nrow: usize) -> Self {
        Self {
            data: VecRecords::new(vec![vec![Text::default(); ncol]; nrow + 1]),
        }
    }

    pub fn count_rows(&self) -> usize {
        self.data.count_rows()
    }

    pub fn count_columns(&self) -> usize {
        self.data.count_columns()
    }

    pub fn put(&mut self, pos: Position, text: String) {
        self.data[pos.row][pos.col] = Text::new(text)
    }

    pub fn draw(self, termwidth: usize) -> Option<String> {
        if self.count_columns() == 0 || self.count_rows() == 0 {
            return Some(Default::default());
        }

        let data: Vec<Vec<_>> = self.data.into();
        let mut table = Builder::from(data).build();
        table.with(Style::sharp());
        table.modify(
            Segment::all(),
            Settings::new(Alignment::left(), Alignment::top()),
        );

        table.with((
            Width::wrap(termwidth).priority(PriorityMax::default()),
            Width::increase(termwidth).priority(PriorityMin::default()),
        ));
        Some(table.to_string())
    }
}

/// Output format for rendering query results.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Pretty-printed ASCII table (default).
    #[default]
    Table,
    /// JSON array of row objects.
    Json,
    /// Comma-separated values with a header row.
    Csv,
}

fn ele_to_string(ele: &Ele) -> String {
    match ele {
        Ele::Nil => "nil".to_string(),
        Ele::BOOL(x) => x.to_string(),
        Ele::I32(x) => x.to_string(),
        Ele::I64(x) => x.to_string(),
        Ele::F32(x) => x.to_string(),
        Ele::F64(x) => x.to_string(),
        Ele::Text(x) => x.to_string(),
        Ele::Url(x) => x.to_string(),
        Ele::DataTime(x) => x.to_string(),
    }
}

/// Render a [`DataFrame`] using the requested [`OutputFormat`].
pub fn render(df: &DataFrame, format: OutputFormat) {
    match format {
        OutputFormat::Table => render_dataframe(df),
        OutputFormat::Json => println!("{}", render_json(df)),
        OutputFormat::Csv => print!("{}", render_csv(df)),
    }
}

/// Serialize a [`DataFrame`] into a JSON array of row objects.
pub fn render_json(df: &DataFrame) -> String {
    let nrow = df.cols.iter().map(|col| col.len()).max().unwrap_or(0);
    let mut rows = Vec::with_capacity(nrow);
    for row in 0..nrow {
        let mut obj = serde_json::Map::new();
        for (col, name) in df.names.iter().enumerate() {
            let value = match df.cols.get(col).and_then(|c| {
                if row < c.len() {
                    Some(c.get(row))
                } else {
                    None
                }
            }) {
                Some(Ele::Nil) | None => serde_json::Value::Null,
                Some(Ele::BOOL(x)) => serde_json::Value::Bool(x),
                Some(Ele::I32(x)) => serde_json::Value::from(x),
                Some(Ele::I64(x)) => serde_json::Value::from(x),
                Some(Ele::F32(x)) => serde_json::Value::from(x),
                Some(Ele::F64(x)) => serde_json::Value::from(x),
                Some(other) => serde_json::Value::String(ele_to_string(&other)),
            };
            obj.insert(name.clone(), value);
        }
        rows.push(serde_json::Value::Object(obj));
    }
    serde_json::to_string_pretty(&serde_json::Value::Array(rows))
        .unwrap_or_else(|_| "[]".to_string())
}

fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Serialize a [`DataFrame`] into CSV text with a header row.
pub fn render_csv(df: &DataFrame) -> String {
    let nrow = df.cols.iter().map(|col| col.len()).max().unwrap_or(0);
    let mut out = String::new();
    out.push_str(
        &df.names
            .iter()
            .map(|n| csv_escape(n))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');
    for row in 0..nrow {
        let cells: Vec<String> = df
            .cols
            .iter()
            .map(|c| {
                if row < c.len() {
                    csv_escape(&ele_to_string(&c.get(row)))
                } else {
                    String::new()
                }
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

pub fn render_dataframe(df: &DataFrame) {
    let ncol = df.names.len();
    let nrow = df.cols.iter().map(|col| col.len()).max().unwrap_or(0);

    let mut table = Table::new(ncol, nrow);

    for (col, name) in df.names.iter().enumerate() {
        table.put((0_usize, col).into(), name.clone());
    }

    for (col, col_data) in df.cols.iter().enumerate() {
        for row in 0..col_data.len() {
            let value = match col_data.get(row) {
                Ele::Nil => "nil".to_string(),
                Ele::BOOL(x) => x.to_string(),
                Ele::I32(x) => x.to_string(),
                Ele::I64(x) => x.to_string(),
                Ele::F32(x) => x.to_string(),
                Ele::F64(x) => x.to_string(),
                Ele::Text(x) => x.to_string(),
                Ele::Url(x) => x.to_string(),
                Ele::DataTime(x) => x.to_string(),
            };
            table.put((row + 1, col).into(), value);
        }
    }
    println!(
        "{}",
        table.draw(terminal_width().unwrap_or(80) as usize).unwrap()
    );
}

fn terminal_width() -> Option<u32> {
    #[cfg(unix)]
    {
        terminal_size_of(std::io::stdout())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(unix)]
ioctl_read!(get_winsize, libc::TIOCGWINSZ, 0, libc::winsize);

#[cfg(unix)]
fn terminal_size_of<Fd: AsFd>(fd: Fd) -> Option<u32> {
    use nix::unistd::isatty;
    if isatty(fd.as_fd()).is_err() {
        return None;
    }

    let winsize = unsafe {
        let mut winsize = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        get_winsize(fd.as_fd().as_raw_fd(), &mut winsize).ok()?;
        winsize
    };
    let cols = winsize.ws_col;

    if cols > 0 {
        Some(cols as u32)
    } else {
        None
    }
}

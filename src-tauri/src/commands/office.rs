//! Office documents in Quick View. Spreadsheets are read here with calamine
//! (xlsx/xlsm/xlsb/ods) and sent to the frontend as a grid of display
//! strings. LibreOffice, when installed, converts any Office file
//! to PDF for the existing PDF viewer: always for .doc/.ppt/.xls, and for
//! the modern formats too when the user opts in. .docx and .pptx otherwise
//! render in the frontend.

use super::preview::read_whole;
use super::{blocking, require_absolute};
use crate::lru::Lru;
use crate::state::{AppState, Location, Resolved};
use calamine::{Data, DataRef, Reader, Sheets, XlsxError};
use serde::Serialize;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;
use tauri::ipc::Response;

/// Biggest workbook read into memory for the grid.
const SHEET_FILE_MAX: u64 = 30 * 1024 * 1024;
/// Most a zipped workbook (xlsx, xlsb, ods) may unpack to. calamine reads
/// whole parts into memory (the shared strings, an .ods content.xml), and
/// 30 MiB of zip can hold gigabytes of them.
const SHEET_UNPACKED_MAX: u64 = 512 * 1024 * 1024;
/// Sheets read per workbook, and characters shown per cell.
const SHEETS_MAX: usize = 100;
const CELL_CHARS_MAX: usize = 4096;
/// Rows and columns sent per sheet; the rest is summarized as a count.
const SHEET_ROWS_MAX: usize = 2000;
const SHEET_COLS_MAX: usize = 100;
/// Biggest remote/archived file copied out for LibreOffice.
const CONVERT_INPUT_MAX: u64 = 50 * 1024 * 1024;
/// Biggest PDF read back from a conversion.
const CONVERT_OUTPUT_MAX: u64 = 150 * 1024 * 1024;
/// A fresh profile's first start alone can take ten seconds or more.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(90);
/// soffice's own console chatter, not the PDF (that goes to a file).
const CONVERT_LOG_MAX: u64 = 1024 * 1024;

/// LibreOffice conversions: finished PDFs, and the one-at-a-time gate.
#[derive(Default)]
pub struct OfficeState {
    /// PDFs keyed by hash(location, path, size, mtime, soffice). Few
    /// entries: each can be megabytes.
    cache: Mutex<Lru<u64, Arc<Vec<u8>>, 8>>,
    /// One soffice at a time: a profile directory can't be shared by two
    /// running instances (the second would hand its job to the first).
    gate: Mutex<()>,
    /// Bumped per request; a request that waited behind a newer one gives
    /// up instead of converting a file the cursor has already left.
    generation: AtomicU64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
    /// Size of the sheet's used range before the cap.
    pub total_rows: usize,
    pub total_cols: usize,
    /// Zero-based position of the range's top-left cell, so headers show
    /// the real row numbers and column letters.
    pub first_row: u32,
    pub first_col: u32,
    /// A sheet that could not be read (the others still show).
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
}

/// A cell as Excel would roughly show it without its number format.
fn cell_text(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => {
            match s.char_indices().nth(CELL_CHARS_MAX) {
                Some((end, _)) => format!("{}…", &s[..end]),
                None => s.clone(),
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Float(f) => float_text(*f),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::Error(e) => e.to_string(),
        Data::DateTime(dt) => {
            if dt.is_duration() {
                let secs = (dt.as_f64() * 86_400.0).round() as i64;
                let sign = if secs < 0 { "-" } else { "" };
                let secs = secs.abs();
                format!("{sign}{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
            } else {
                let (y, mo, d, h, mi, s, _) = dt.to_ymd_hms_milli();
                let time = (h, mi, s) != (0, 0, 0);
                if dt.as_f64() < 1.0 {
                    // A bare time of day (serial date 0).
                    format!("{h:02}:{mi:02}:{s:02}")
                } else if time {
                    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
                } else {
                    format!("{y:04}-{mo:02}-{d:02}")
                }
            }
        }
    }
}

/// Shortest readable form: whole numbers without ".0", float noise such
/// as 0.30000000000000004 rounded away (Excel itself shows 15 digits).
fn float_text(f: f64) -> String {
    if !f.is_finite() {
        return f.to_string();
    }
    if f.fract() == 0.0 && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    let s = format!("{:.10}", f);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".to_string() } else { s.to_string() }
}

/// The capped grid of one sheet's used range.
fn sheet_from_range(name: String, range: &calamine::Range<Data>) -> Sheet {
    let (total_rows, total_cols) = range.get_size();
    let (first_row, first_col) = range.start().unwrap_or((0, 0));
    let rows = range
        .rows()
        .take(SHEET_ROWS_MAX)
        .map(|row| row.iter().take(SHEET_COLS_MAX).map(cell_text).collect())
        .collect();
    Sheet { name, rows, total_rows, total_cols, first_row, first_col, error: None }
}

/// The capped grid of a sheet whose cells arrive one at a time, in any
/// order. calamine's own range is a dense grid of the whole used area, and
/// a file of a few hundred bytes can claim the area A1:XFD1048576. Only the
/// cells that can end up in the capped grid are kept here; the area is just
/// measured.
#[derive(Default)]
struct Window {
    cells: std::collections::HashMap<(u32, u32), String>,
    /// (first, last) used position, once there is a cell.
    bounds: Option<((u32, u32), (u32, u32))>,
}

impl Window {
    fn shows(first: (u32, u32), (row, col): (u32, u32)) -> bool {
        ((row - first.0) as usize) < SHEET_ROWS_MAX && ((col - first.1) as usize) < SHEET_COLS_MAX
    }

    fn add(&mut self, pos: (u32, u32), value: &DataRef) {
        if matches!(value, DataRef::Empty) {
            return;
        }
        let (first, last) = self.bounds.unwrap_or((pos, pos));
        let first = (first.0.min(pos.0), first.1.min(pos.1));
        let last = (last.0.max(pos.0), last.1.max(pos.1));
        self.bounds = Some((first, last));
        // The top-left corner only ever moves up and left, so a cell
        // outside the shown part now stays outside it.
        if !Self::shows(first, pos) {
            return;
        }
        self.cells.insert(pos, cell_text(&value.clone().into()));
        if self.cells.len() > 2 * SHEET_ROWS_MAX * SHEET_COLS_MAX {
            self.cells.retain(|&pos, _| Self::shows(first, pos));
        }
    }

    fn into_sheet(self, name: String) -> Sheet {
        let Some((first, last)) = self.bounds else {
            return sheet_from_range(name, &calamine::Range::empty());
        };
        let total_rows = (last.0 - first.0) as usize + 1;
        let total_cols = (last.1 - first.1) as usize + 1;
        let mut rows =
            vec![vec![String::new(); total_cols.min(SHEET_COLS_MAX)]; total_rows.min(SHEET_ROWS_MAX)];
        for (pos, text) in self.cells {
            if Self::shows(first, pos) {
                rows[(pos.0 - first.0) as usize][(pos.1 - first.1) as usize] = text;
            }
        }
        Sheet {
            name,
            rows,
            total_rows,
            total_cols,
            first_row: first.0,
            first_col: first.1,
            error: None,
        }
    }
}

/// One sheet as a capped grid. xlsx and xlsb are streamed through a
/// [`Window`]; ods has no cell reader in calamine and comes as its range,
/// bounded by calamine's own cell limit.
fn read_sheet<RS: std::io::Read + std::io::Seek>(
    book: &mut Sheets<RS>,
    name: String,
) -> Result<Sheet, String> {
    let mut window = Window::default();
    match book {
        Sheets::Xlsx(xlsx) => match xlsx.worksheet_cells_reader(&name) {
            Ok(mut cells) => {
                while let Some(cell) = cells.next_cell().map_err(|e| e.to_string())? {
                    window.add(cell.get_position(), cell.get_value());
                }
            }
            // A chart sheet: listed, with nothing to show.
            Err(XlsxError::NotAWorksheet(_)) => {}
            Err(e) => return Err(e.to_string()),
        },
        Sheets::Xlsb(xlsb) => {
            let mut cells = xlsb.worksheet_cells_reader(&name).map_err(|e| e.to_string())?;
            while let Some(cell) = cells.next_cell().map_err(|e| e.to_string())? {
                window.add(cell.get_position(), cell.get_value());
            }
        }
        _ => {
            let range = book.worksheet_range(&name).map_err(|e| e.to_string())?;
            return Ok(sheet_from_range(name, &range));
        }
    }
    Ok(window.into_sheet(name))
}

/// How an OLE2 compound file starts: the legacy binary formats.
const OLE2_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

fn read_workbook(bytes: Vec<u8>) -> Result<Workbook, String> {
    // calamine reads a whole .xls into dense grids while opening it, with
    // no way to cap them, and goes by content: the file's name is no guard.
    // Legacy workbooks are LibreOffice's to convert.
    if bytes.starts_with(&OLE2_MAGIC) {
        return Err("Legacy .xls workbooks need LibreOffice".to_string());
    }
    // What the zip says it unpacks to, checked before calamine unpacks it.
    if let Ok(zip) = zip::ZipArchive::new(std::io::Cursor::new(&bytes)) {
        let unpacked = (0..zip.len())
            .filter_map(|i| zip.by_index_data(i).ok().map(|e| e.size()))
            .fold(0u64, u64::saturating_add);
        if unpacked > SHEET_UNPACKED_MAX {
            return Err("Workbook too large to preview".to_string());
        }
    }
    let mut book = calamine::open_workbook_auto_from_rs(std::io::Cursor::new(bytes))
        .map_err(|e| format!("Not a readable spreadsheet: {e}"))?;
    let names = book.sheet_names().to_owned();
    let sheets = names
        .into_iter()
        .take(SHEETS_MAX)
        .map(|name| match read_sheet(&mut book, name.clone()) {
            Ok(sheet) => sheet,
            Err(error) => Sheet {
                name,
                rows: Vec::new(),
                total_rows: 0,
                total_cols: 0,
                first_row: 0,
                first_col: 0,
                error: Some(error),
            },
        })
        .collect();
    Ok(Workbook { sheets })
}

/// Every sheet of a workbook as display text (Quick View's table).
#[tauri::command]
pub async fn office_sheets(
    state: State<'_, AppState>,
    location: Location,
    path: PathBuf,
) -> Result<Workbook, String> {
    let resolved = state.resolve(&location)?;
    blocking(move || read_workbook(read_whole(resolved, &path, SHEET_FILE_MAX)?)).await
}

#[derive(Serialize)]
pub struct OfficeTools {
    /// The soffice binary Quick View would use, if any.
    pub soffice: Option<String>,
}

/// Whether LibreOffice is available for Office previews. The caller passes
/// the Settings path itself: a just-edited value may not have reached the
/// backend's config yet.
#[tauri::command]
pub async fn office_tools(soffice_path: String) -> Result<OfficeTools, String> {
    let custom = soffice_path;
    blocking(move || {
        Ok(OfficeTools {
            soffice: crate::platform::find_soffice(&custom)
                .map(|p| p.to_string_lossy().into_owned()),
        })
    })
    .await
}

/// Arguments for one headless conversion of `input` into `outdir`.
fn convert_args(profile: &Path, outdir: &Path, input: &Path) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = [
        "--headless",
        "--norestore",
        "--nolockcheck",
        "--nodefault",
        "--nologo",
    ]
    .iter()
    .map(Into::into)
    .collect();
    args.push(format!("-env:UserInstallation={}", crate::platform::file_uri(profile)).into());
    args.push("--convert-to".into());
    args.push("pdf".into());
    args.push("--outdir".into());
    args.push(outdir.into());
    args.push(input.into());
    args
}

/// Owner-only scratch directory, removed with everything in it on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "yc-office-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The private LibreOffice profile Quick View converts with.
fn profile_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("your-commander")
        .join("lo-profile")
}

/// Run soffice on `input` and return the PDF it writes.
fn run_soffice(soffice: &Path, input: &Path, scratch: &Path) -> Result<Vec<u8>, String> {
    let outdir = scratch.join("out");
    std::fs::create_dir(&outdir).map_err(|e| format!("{}: {e}", outdir.display()))?;
    let child = std::process::Command::new(soffice)
        .args(convert_args(&profile_dir(), &outdir, input))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("LibreOffice: {e}"))?;
    let output = crate::proc::wait_bounded(child, CONVERT_TIMEOUT, CONVERT_LOG_MAX)
        .map_err(|e| format!("LibreOffice: {e}"))?;
    let pdf = std::fs::read_dir(&outdir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pdf")));
    let Some(pdf) = pdf else {
        let log = String::from_utf8_lossy(&output.stderr);
        let log = log.trim();
        return Err(if log.is_empty() {
            "LibreOffice could not convert this file".to_string()
        } else {
            format!("LibreOffice: {log}")
        });
    };
    if std::fs::metadata(&pdf).map(|m| m.len()).unwrap_or(0) > CONVERT_OUTPUT_MAX {
        return Err("The converted PDF is too big to preview".into());
    }
    std::fs::read(&pdf).map_err(|e| e.to_string())
}

fn cache_key(location: &Location, path: &Path, size: u64, modified: i64, soffice: &Path) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    serde_json::to_string(location).unwrap_or_default().hash(&mut hasher);
    (path, size, modified, soffice).hash(&mut hasher);
    hasher.finish()
}

/// True when a newer request arrived while this one waited.
fn superseded(generation: &AtomicU64, mine: u64) -> bool {
    generation.load(Ordering::SeqCst) != mine
}

/// An Office file converted to PDF by LibreOffice. `size` and `modified`
/// (the listing's values) key the cache, so an edited file converts anew.
/// Fails with "superseded" when a newer conversion was requested meanwhile.
#[tauri::command]
pub async fn office_to_pdf(
    state: State<'_, AppState>,
    location: Location,
    path: PathBuf,
    size: u64,
    modified: i64,
) -> Result<Response, String> {
    let custom = state.config.lock().unwrap().office_soffice.clone();
    let resolved = state.resolve(&location)?;
    let soffice = blocking(move || {
        crate::platform::find_soffice(&custom).ok_or_else(|| "LibreOffice not found".to_string())
    })
    .await?;
    let key = cache_key(&location, &path, size, modified, &soffice);
    if let Some(pdf) = state.office.cache.lock().unwrap().get(&key) {
        return Ok(Response::new(pdf.as_ref().clone()));
    }
    let mine = state.office.generation.fetch_add(1, Ordering::SeqCst) + 1;

    let office = state.office.clone();
    let pdf = blocking(move || -> Result<Vec<u8>, String> {
        let _turn = office.gate.lock().unwrap();
        if superseded(&office.generation, mine) {
            return Err("superseded".into());
        }
        let scratch = Scratch::new()?;
        let input = match resolved {
            Resolved::Local => {
                require_absolute(&path)?;
                path.clone()
            }
            remote => {
                // Copied out under a fixed name: only the extension matters
                // to LibreOffice, and the remote name never reaches its
                // command line.
                let bytes = read_whole(remote, &path, CONVERT_INPUT_MAX)?;
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
                    .unwrap_or_default();
                let local = scratch.0.join(format!("document.{ext}"));
                std::fs::write(&local, bytes).map_err(|e| e.to_string())?;
                local
            }
        };
        run_soffice(&soffice, &input, &scratch.0)
    })
    .await?;

    let pdf = Arc::new(pdf);
    state.office.cache.lock().unwrap().insert(key, pdf.clone());
    Ok(Response::new(pdf.as_ref().clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::{CellErrorType, Range};

    #[test]
    fn cells_read_like_excel() {
        assert_eq!(cell_text(&Data::Empty), "");
        assert_eq!(cell_text(&Data::Int(42)), "42");
        assert_eq!(cell_text(&Data::Float(3.0)), "3");
        assert_eq!(cell_text(&Data::Float(0.1 + 0.2)), "0.3");
        assert_eq!(cell_text(&Data::Float(-1.25)), "-1.25");
        assert_eq!(cell_text(&Data::Bool(true)), "TRUE");
        assert_eq!(cell_text(&Data::Error(CellErrorType::Div0)), "#DIV/0!");
        assert_eq!(cell_text(&Data::String("a".into())), "a");
    }

    #[test]
    fn sheets_are_capped_and_keep_their_origin() {
        let mut range: Range<Data> = Range::new((2, 1), (2 + 2500, 1 + 150));
        range.set_value((2, 1), Data::String("top-left".into()));
        let sheet = sheet_from_range("S".into(), &range);
        assert_eq!(sheet.rows.len(), SHEET_ROWS_MAX);
        assert!(sheet.rows.iter().all(|r| r.len() == SHEET_COLS_MAX));
        assert_eq!((sheet.total_rows, sheet.total_cols), (2501, 151));
        assert_eq!((sheet.first_row, sheet.first_col), (2, 1));
        assert_eq!(sheet.rows[0][0], "top-left");
    }

    /// A workbook with one sheet holding `sheet_data` (the inside of
    /// `<sheetData>`).
    fn xlsx(sheet_data: &str) -> Vec<u8> {
        use std::io::Write;
        const NS: &str = "http://schemas.openxmlformats.org";
        let parts = [
            (
                "[Content_Types].xml",
                format!(
                    r#"<Types xmlns="{NS}/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#
                ),
            ),
            (
                "_rels/.rels",
                format!(
                    r#"<Relationships xmlns="{NS}/package/2006/relationships"><Relationship Id="rId1" Type="{NS}/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
                ),
            ),
            (
                "xl/workbook.xml",
                format!(
                    r#"<workbook xmlns="{NS}/spreadsheetml/2006/main" xmlns:r="{NS}/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#
                ),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                format!(
                    r#"<Relationships xmlns="{NS}/package/2006/relationships"><Relationship Id="rId1" Type="{NS}/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
                ),
            ),
            (
                "xl/worksheets/sheet1.xml",
                format!(
                    r#"<worksheet xmlns="{NS}/spreadsheetml/2006/main"><sheetData>{sheet_data}</sheetData></worksheet>"#
                ),
            ),
        ];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, xml) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// Two cells at opposite corners of the biggest sheet there is: a dense
    /// grid of that area would be 17 billion cells.
    #[test]
    fn a_sheet_claiming_a_huge_area_costs_only_its_shown_part() {
        let book = read_workbook(xlsx(
            r#"<row r="1048576"><c r="XFD1048576"><v>9</v></c></row><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c></row>"#,
        ))
        .unwrap();
        let sheet = &book.sheets[0];
        assert_eq!(sheet.error, None);
        assert_eq!((sheet.total_rows, sheet.total_cols), (1_048_576, 16_384));
        assert_eq!((sheet.first_row, sheet.first_col), (0, 0));
        assert_eq!(sheet.rows.len(), SHEET_ROWS_MAX);
        assert!(sheet.rows.iter().all(|r| r.len() == SHEET_COLS_MAX));
        assert_eq!(sheet.rows[0][..3], ["1", "2", ""]);
    }

    /// Same grid as calamine's range gives: it starts at the first used
    /// cell, wherever in the file that cell comes.
    #[test]
    fn streamed_sheets_keep_their_origin() {
        let book = read_workbook(xlsx(
            r#"<row r="5"><c r="D5"><v>4</v></c></row><row r="3"><c r="C3"><v>3</v></c><c r="E3"><v>5</v></c></row>"#,
        ))
        .unwrap();
        let sheet = &book.sheets[0];
        assert_eq!((sheet.first_row, sheet.first_col), (2, 2));
        assert_eq!((sheet.total_rows, sheet.total_cols), (3, 3));
        assert_eq!(sheet.rows, [["3", "", "5"], ["", "", ""], ["", "4", ""]]);

        let empty = read_workbook(xlsx("")).unwrap();
        assert!(empty.sheets[0].rows.is_empty());
        assert_eq!(empty.sheets[0].total_rows, 0);
    }

    #[test]
    fn not_a_spreadsheet_is_an_error() {
        assert!(read_workbook(b"hello".to_vec()).is_err());
        // An .xls, whatever it is named: refused before calamine opens it.
        let mut xls = OLE2_MAGIC.to_vec();
        xls.resize(4096, 0);
        assert!(read_workbook(xls).is_err_and(|e| e.contains("LibreOffice")));
    }

    #[test]
    fn soffice_gets_a_private_profile_url_and_output_dir() {
        let args = convert_args(
            Path::new("/Users/a b/Library/Caches/your-commander/lo-profile"),
            Path::new("/tmp/out"),
            Path::new("/data/x.doc"),
        );
        let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"--headless".to_string()));
        assert!(args.contains(
            &"-env:UserInstallation=file:///Users/a%20b/Library/Caches/your-commander/lo-profile"
                .to_string()
        ));
        let n = args.len();
        assert_eq!(args[n - 3..], ["--outdir", "/tmp/out", "/data/x.doc"]);
    }

    #[test]
    fn a_newer_request_supersedes_a_waiting_one() {
        let generation = AtomicU64::new(0);
        let first = generation.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(!superseded(&generation, first));
        let second = generation.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(superseded(&generation, first));
        assert!(!superseded(&generation, second));
    }

    /// `cargo test -- --ignored` on a machine with LibreOffice installed.
    #[test]
    #[ignore = "needs LibreOffice"]
    fn libreoffice_converts_to_pdf() {
        let soffice = crate::platform::find_soffice("").expect("LibreOffice not installed");
        let scratch = Scratch::new().unwrap();
        let input = scratch.0.join("hello.rtf");
        std::fs::write(&input, r"{\rtf1\ansi Hello from Quick View\par}").unwrap();
        let pdf = run_soffice(&soffice, &input, &scratch.0).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn cache_key_follows_file_changes() {
        let loc = Location::Local;
        let soffice = Path::new("/x/soffice");
        let p = Path::new("/d/a.doc");
        let k = cache_key(&loc, p, 10, 100, soffice);
        assert_eq!(k, cache_key(&loc, p, 10, 100, soffice));
        assert_ne!(k, cache_key(&loc, p, 11, 100, soffice));
        assert_ne!(k, cache_key(&loc, p, 10, 101, soffice));
    }
}

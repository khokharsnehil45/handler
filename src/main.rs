use clap::Parser;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Cell, Color, Table};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Seek;
use std::path::PathBuf;
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "handler",
    version,
    about = "A fast and simple CLI tool to search CSV files for missing values, type mismatches, outliers, and duplicates — and surgically repair or auto-clean them"
)]
struct Cli {
    /// Path to the CSV file (supports --file_path or --file)
    #[arg(short = 'f', long = "file", alias = "file_path", value_name = "PATH")]
    file_path: Option<PathBuf>,

    /// Positional path to the CSV file
    #[arg(value_name = "FILE")]
    positional_path: Option<PathBuf>,

    /// Search for missing values
    #[arg(
        short = 's',
        long = "search_missing",
        alias = "search-missing",
        alias = "search"
    )]
    search_missing: bool,

    /// Search for data type mismatches
    #[arg(
        short = 't',
        long = "search_types",
        alias = "check_types",
        alias = "check-types",
        alias = "type_mismatch",
        alias = "type-mismatch"
    )]
    search_types: bool,

    /// Search for numerical outliers using IQR method
    #[arg(
        short = 'o',
        long = "search_outliers",
        alias = "check_outliers",
        alias = "check-outliers",
        alias = "outliers"
    )]
    search_outliers: bool,

    /// Search for duplicate rows
    #[arg(
        short = 'd',
        long = "search_duplicates",
        alias = "check_duplicates",
        alias = "check-duplicates",
        alias = "duplicates"
    )]
    search_duplicates: bool,

    /// Key column to check duplicates against (e.g. --key user_id). If omitted, checks entire rows
    #[arg(short = 'k', long = "key", value_name = "COLUMN")]
    key: Option<String>,

    /// Run full audit (missing values, type mismatches, outliers, and duplicates)
    #[arg(short = 'a', long = "audit")]
    audit: bool,

    /// Output results as JSON for agent tools and automated pipelines
    #[arg(short = 'j', long = "json")]
    json: bool,

    /// Show detailed rows with issues
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    // --- SURGICAL PINPOINT PATCHING ---
    /// Surgically patch a cell by row and column (e.g. --patch "3:age=22" or --patch "5:signup_date=2026-03-01")
    #[arg(long = "patch", value_name = "ROW:COL=VALUE")]
    patch: Vec<String>,

    /// Surgically drop a specific row number (e.g. --drop-row 10)
    #[arg(long = "drop-row", alias = "drop_row", value_name = "ROW_NUM")]
    drop_row: Vec<usize>,

    /// Save modifications in-place to the original file
    #[arg(long = "in-place", alias = "in_place")]
    in_place: bool,

    // --- REPAIR / REMEDIATION FLAGS ---
    /// Auto-repair/clean the CSV and export a sanitized copy
    #[arg(long = "fix", alias = "repair", alias = "clean")]
    fix: bool,

    /// Destination file for the cleaned CSV (defaults to <original>_clean.csv)
    #[arg(short = 'O', long = "output", value_name = "OUTPUT_PATH")]
    output: Option<PathBuf>,

    /// Drop duplicate rows (keeps first occurrence)
    #[arg(long = "drop-duplicates", alias = "drop_duplicates")]
    drop_duplicates: bool,

    /// Drop rows with invalid / unparseable type mismatches
    #[arg(long = "drop-invalid", alias = "drop_invalid")]
    drop_invalid: bool,

    /// Drop rows containing statistical outliers
    #[arg(long = "drop-outliers", alias = "drop_outliers")]
    drop_outliers: bool,

    /// Drop rows containing missing values
    #[arg(long = "drop-missing", alias = "drop_missing")]
    drop_missing: bool,

    /// Fill missing values (impute median for numeric, empty for text)
    #[arg(long = "fill-missing", alias = "fill_missing")]
    fill_missing: bool,

    /// Coerce types (e.g. floats to whole integers in int columns, yes/no to true/false)
    #[arg(long = "coerce-types", alias = "coerce_types")]
    coerce_types: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum DataType {
    Integer,
    Float,
    Boolean,
    Date,
    Text,
}

impl DataType {
    fn name(&self) -> &'static str {
        match self {
            DataType::Integer => "Integer",
            DataType::Float => "Float",
            DataType::Boolean => "Boolean",
            DataType::Date => "Date",
            DataType::Text => "Text",
        }
    }

    fn is_numeric(&self) -> bool {
        matches!(self, DataType::Integer | DataType::Float)
    }
}

#[derive(Default, Clone)]
struct ColTypeStats {
    total_non_missing: usize,
    int_count: usize,
    float_count: usize,
    bool_count: usize,
    date_count: usize,
}

#[derive(Clone, Debug)]
struct OutlierBounds {
    q1: f64,
    q3: f64,
    iqr: f64,
    lower_bound: f64,
    upper_bound: f64,
    min: f64,
    max: f64,
}

#[derive(Serialize)]
struct FullReportJson {
    file_path: String,
    total_rows: usize,
    total_columns: usize,
    headers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing_values: Option<MissingReportJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    type_validation: Option<TypeReportJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outliers: Option<OutlierReportJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicates: Option<DuplicateReportJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repair: Option<RepairReportJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    surgical_patch: Option<PatchReportJson>,
}

#[derive(Serialize)]
struct MissingReportJson {
    total_missing_values: usize,
    rows_with_missing: usize,
    rows_with_missing_pct: f64,
    columns: Vec<MissingColJson>,
    affected_rows: Vec<MissingRowSampleJson>,
}

#[derive(Serialize)]
struct MissingColJson {
    index: usize,
    name: String,
    missing_count: usize,
    missing_percent: f64,
    is_clean: bool,
}

#[derive(Serialize)]
struct MissingRowSampleJson {
    row: usize,
    missing_columns: Vec<String>,
}

#[derive(Serialize)]
struct TypeReportJson {
    total_mismatches: usize,
    columns: Vec<TypeColJson>,
    mismatch_details: Vec<TypeMismatchDetailJson>,
}

#[derive(Serialize)]
struct TypeColJson {
    index: usize,
    name: String,
    inferred_type: String,
    valid_cells: usize,
    mismatches: usize,
    is_clean: bool,
}

#[derive(Serialize)]
struct TypeMismatchDetailJson {
    row: usize,
    column: String,
    expected_type: String,
    found_value: String,
    reason: String,
}

#[derive(Serialize)]
struct OutlierReportJson {
    method: String,
    total_outliers: usize,
    columns: Vec<OutlierColJson>,
    outlier_details: Vec<OutlierDetailJson>,
}

#[derive(Serialize)]
struct OutlierColJson {
    index: usize,
    name: String,
    inferred_type: String,
    is_numeric: bool,
    lower_bound: Option<f64>,
    upper_bound: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    outliers: usize,
    is_clean: bool,
}

#[derive(Serialize)]
struct OutlierDetailJson {
    row: usize,
    column: String,
    value: f64,
    lower_bound: f64,
    upper_bound: f64,
    reason: String,
}

#[derive(Serialize)]
struct DuplicateReportJson {
    scope: String,
    total_rows: usize,
    unique_records: usize,
    duplicate_rows: usize,
    duplicate_percent: f64,
    clusters: Vec<DuplicateClusterJson>,
}

#[derive(Serialize)]
struct DuplicateClusterJson {
    cluster_index: usize,
    description: String,
    occurrences: usize,
    row_numbers: Vec<usize>,
}

#[derive(Serialize)]
struct RepairReportJson {
    input_file: String,
    output_file: String,
    input_rows: usize,
    output_rows: usize,
    total_rows_dropped: usize,
    duplicates_dropped: usize,
    invalid_rows_dropped: usize,
    outliers_dropped: usize,
    missing_rows_dropped: usize,
    cells_imputed: usize,
    types_coerced: usize,
    status: String,
}

#[derive(Serialize, Clone)]
struct PatchAppliedJson {
    row: usize,
    column: String,
    old_value: String,
    new_value: String,
}

#[derive(Serialize)]
struct PatchReportJson {
    input_file: String,
    output_file: String,
    patches_applied: Vec<PatchAppliedJson>,
    rows_dropped: Vec<usize>,
    status: String,
}

fn is_missing_value(val: &str) -> bool {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return true;
    }

    matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "na" | "n/a" | "nan" | "null" | "none" | "nil" | "-" | "?"
    )
}

fn is_bool_val(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no" | "1" | "0" | "t" | "f"
    )
}

fn coerce_bool(s: &str) -> Option<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "t" | "1" => Some("true"),
        "false" | "no" | "f" | "0" => Some("false"),
        _ => None,
    }
}

fn is_int_val(s: &str) -> bool {
    s.parse::<i64>().is_ok()
}

fn coerce_int(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if let Ok(i) = trimmed.parse::<i64>() {
        return Some(i.to_string());
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        return Some(format!("{:.0}", f.round()));
    }
    None
}

fn is_float_val(s: &str) -> bool {
    if is_int_val(s) {
        return false;
    }
    s.parse::<f64>().is_ok()
}

fn is_date_val(s: &str) -> bool {
    let parts: Vec<&str> = if s.contains('-') {
        s.split('-').collect()
    } else if s.contains('/') {
        s.split('/').collect()
    } else {
        return false;
    };

    if parts.len() != 3 {
        return false;
    }

    // Format YYYY-MM-DD or YYYY/MM/DD
    if parts[0].len() == 4 {
        if let (Ok(y), Ok(m), Ok(d)) = (
            parts[0].parse::<u32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u32>(),
        ) {
            return (1000..=9999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d);
        }
    }
    // Format DD-MM-YYYY or MM-DD-YYYY
    if parts[2].len() == 4 {
        if let (Ok(p1), Ok(p2), Ok(y)) = (
            parts[0].parse::<u32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u32>(),
        ) {
            return (1000..=9999).contains(&y) && (1..=31).contains(&p1) && (1..=31).contains(&p2);
        }
    }
    false
}

fn infer_type(stats: &ColTypeStats) -> DataType {
    if stats.total_non_missing == 0 {
        return DataType::Text;
    }

    let n = stats.total_non_missing as f64;
    let int_ratio = stats.int_count as f64 / n;
    let float_combined_ratio = (stats.int_count + stats.float_count) as f64 / n;
    let bool_ratio = stats.bool_count as f64 / n;
    let date_ratio = stats.date_count as f64 / n;

    // Threshold: if >= 60% of non-empty values match a specific type
    if int_ratio >= 0.60 {
        DataType::Integer
    } else if float_combined_ratio >= 0.60 {
        DataType::Float
    } else if bool_ratio >= 0.60 {
        DataType::Boolean
    } else if date_ratio >= 0.60 {
        DataType::Date
    } else {
        DataType::Text
    }
}

fn validate_type_match(val: &str, expected: DataType) -> Result<(), &'static str> {
    let trimmed = val.trim();
    match expected {
        DataType::Integer => {
            if is_int_val(trimmed) {
                Ok(())
            } else if trimmed.parse::<f64>().is_ok() {
                Err("Decimal/Float value in Integer column")
            } else {
                Err("Cannot parse as Integer")
            }
        }
        DataType::Float => {
            if trimmed.parse::<f64>().is_ok() {
                Ok(())
            } else {
                Err("Cannot parse as Float/Number")
            }
        }
        DataType::Boolean => {
            if is_bool_val(trimmed) {
                Ok(())
            } else {
                Err("Cannot parse as Boolean")
            }
        }
        DataType::Date => {
            if is_date_val(trimmed) {
                Ok(())
            } else {
                Err("Invalid Date format")
            }
        }
        DataType::Text => Ok(()),
    }
}

fn calculate_percentile(sorted_vals: &[f64], pct: f64) -> f64 {
    let n = sorted_vals.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return sorted_vals[0];
    }
    let idx = (n - 1) as f64 * pct;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    let frac = idx - lo as f64;
    sorted_vals[lo] + frac * (sorted_vals[hi] - sorted_vals[lo])
}

fn compute_outlier_bounds(sorted_vals: &[f64]) -> Option<OutlierBounds> {
    if sorted_vals.len() < 4 {
        return None;
    }
    let min = sorted_vals[0];
    let max = sorted_vals[sorted_vals.len() - 1];
    let q1 = calculate_percentile(sorted_vals, 0.25);
    let q3 = calculate_percentile(sorted_vals, 0.75);
    let iqr = q3 - q1;
    let lower_bound = q1 - 1.5 * iqr;
    let upper_bound = q3 + 1.5 * iqr;

    Some(OutlierBounds {
        q1,
        q3,
        iqr,
        lower_bound,
        upper_bound,
        min,
        max,
    })
}

fn calculate_median(sorted_vals: &[f64]) -> Option<f64> {
    if sorted_vals.is_empty() {
        return None;
    }
    let n = sorted_vals.len();
    if n % 2 == 1 {
        Some(sorted_vals[n / 2])
    } else {
        Some((sorted_vals[n / 2 - 1] + sorted_vals[n / 2]) / 2.0)
    }
}

fn format_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e12 {
        format!("{:.0}", n)
    } else {
        format!("{:.2}", n)
    }
}

fn hash_record(record: &csv::StringRecord) -> u64 {
    let mut hasher = DefaultHasher::new();
    for field in record.iter() {
        field.hash(&mut hasher);
    }
    hasher.finish()
}

fn main() {
    let cli = Cli::parse();

    let target_path = match cli.file_path.or(cli.positional_path) {
        Some(path) => path,
        None => {
            eprintln!("Error: Please provide a CSV file path via --file_path <PATH> or as an argument.");
            eprintln!("Usage: handler --file_path <PATH> [--patch \"ROW:COL=VAL\"] [--fix] [--json]");
            process::exit(1);
        }
    };

    if !target_path.exists() {
        eprintln!("Error: File '{}' does not exist.", target_path.display());
        process::exit(1);
    }

    let mut file = match File::open(&target_path) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("Error opening '{}': {}", target_path.display(), err);
            process::exit(1);
        }
    };

    let has_patches = !cli.patch.is_empty() || !cli.drop_row.is_empty();

    let is_repair_mode = cli.fix
        || cli.drop_duplicates
        || cli.drop_invalid
        || cli.drop_outliers
        || cli.drop_missing
        || cli.fill_missing
        || cli.coerce_types
        || has_patches
        || cli.output.is_some()
        || cli.in_place;

    // In repair mode, default to smart defaults if specific micro-flags not set
    let (do_drop_dups, do_drop_invalid, do_drop_outliers, do_drop_missing, do_fill_missing, do_coerce) =
        if is_repair_mode && (cli.fix || cli.drop_duplicates || cli.drop_invalid || cli.drop_outliers || cli.drop_missing || cli.fill_missing || cli.coerce_types) {
            let specific = cli.drop_duplicates
                || cli.drop_invalid
                || cli.drop_outliers
                || cli.drop_missing
                || cli.fill_missing
                || cli.coerce_types;
            if specific {
                (
                    cli.drop_duplicates,
                    cli.drop_invalid,
                    cli.drop_outliers,
                    cli.drop_missing,
                    cli.fill_missing,
                    cli.coerce_types,
                )
            } else {
                // Smart defaults for --fix
                (true, true, false, false, false, true)
            }
        } else {
            (false, false, false, false, false, false)
        };

    let (check_missing, check_types, check_outliers, check_duplicates) = if cli.audit {
        (true, true, true, true)
    } else if cli.search_missing
        || cli.search_types
        || cli.search_outliers
        || cli.search_duplicates
        || cli.key.is_some()
    {
        (
            cli.search_missing,
            cli.search_types,
            cli.search_outliers,
            cli.search_duplicates || cli.key.is_some(),
        )
    } else if is_repair_mode {
        // In pure repair mode without check flags, don't run diagnostic display unless asked
        (false, false, false, false)
    } else {
        (true, true, true, true)
    };

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(&file);

    let headers = match rdr.headers() {
        Ok(h) => h.clone(),
        Err(err) => {
            eprintln!("Error reading CSV headers: {}", err);
            process::exit(1);
        }
    };

    let col_count = headers.len();
    if col_count == 0 {
        if cli.json {
            println!("{{\"error\": \"The CSV file appears to have no columns.\"}}");
        } else {
            println!("The CSV file appears to have no columns.");
        }
        return;
    }

    let header_names: Vec<String> = headers.iter().map(|s| s.to_string()).collect();

    // Parse surgical patches: "ROW:COL=VALUE"
    let mut surgical_patches: HashMap<(usize, usize), String> = HashMap::new();
    for patch_str in &cli.patch {
        let parts: Vec<&str> = patch_str.splitn(2, ':').collect();
        if parts.len() != 2 {
            eprintln!(
                "Error: Invalid patch format '{}'. Expected 'ROW:COL=VALUE' (e.g. --patch \"3:age=22\")",
                patch_str
            );
            process::exit(1);
        }

        let row_num = match parts[0].trim().parse::<usize>() {
            Ok(r) if r >= 1 => r,
            _ => {
                eprintln!("Error: Invalid row number '{}' in patch '{}'. Row must be >= 1", parts[0], patch_str);
                process::exit(1);
            }
        };

        let col_val_parts: Vec<&str> = parts[1].splitn(2, '=').collect();
        if col_val_parts.len() != 2 {
            eprintln!(
                "Error: Missing '=' in patch '{}'. Expected 'ROW:COL=VALUE'",
                patch_str
            );
            process::exit(1);
        }

        let col_spec = col_val_parts[0].trim();
        let new_val = col_val_parts[1].trim().to_string();

        let col_idx = if let Ok(idx) = col_spec.parse::<usize>() {
            if idx >= 1 && idx <= col_count {
                idx - 1
            } else {
                eprintln!(
                    "Error: Column index '{}' out of bounds (1..{})",
                    idx, col_count
                );
                process::exit(1);
            }
        } else if let Some(pos) = header_names.iter().position(|h| h.eq_ignore_ascii_case(col_spec)) {
            pos
        } else {
            eprintln!(
                "Error: Column '{}' not found. Available columns: {}",
                col_spec,
                header_names.join(", ")
            );
            process::exit(1);
        };

        surgical_patches.insert((row_num, col_idx), new_val);
    }

    let pinpoint_drop_rows: HashSet<usize> = cli.drop_row.iter().copied().collect();

    // Verify key column if specified
    let key_col_idx = if let Some(ref k) = cli.key {
        match header_names
            .iter()
            .position(|h| h.eq_ignore_ascii_case(k.trim()))
        {
            Some(idx) => Some(idx),
            None => {
                eprintln!(
                    "Error: Key column '{}' not found in CSV. Available headers: {}",
                    k,
                    header_names.join(", ")
                );
                process::exit(1);
            }
        }
    } else {
        None
    };

    let mut missing_counts = vec![0usize; col_count];
    let mut type_stats = vec![ColTypeStats::default(); col_count];
    let mut numeric_values: Vec<Vec<f64>> = vec![Vec::new(); col_count];
    let mut total_rows = 0usize;
    let mut rows_with_missing = 0usize;
    let mut missing_row_samples: Vec<(usize, Vec<String>)> = Vec::new();

    // Duplicate detection storage
    let mut row_hashes: HashMap<u64, Vec<(usize, Vec<String>)>> = HashMap::new();
    let mut key_occurrences: HashMap<String, Vec<usize>> = HashMap::new();
    let mut duplicate_row_indices: HashSet<usize> = HashSet::new();

    // Pass 1: Missing values, type statistics, numeric values, and duplicates
    for (row_idx, result) in rdr.records().enumerate() {
        let record = match result {
            Ok(rec) => rec,
            Err(err) => {
                eprintln!("Warning: Skipping record at row {}: {}", row_idx + 1, err);
                continue;
            }
        };

        total_rows += 1;
        let mut row_has_missing = false;
        let mut missing_cols_in_row = Vec::new();

        // 1. Missing values & Type stats collection
        for (col_idx, field) in record.iter().enumerate() {
            let is_missing = is_missing_value(field);

            if col_idx < col_count {
                if is_missing {
                    missing_counts[col_idx] += 1;
                    row_has_missing = true;
                    missing_cols_in_row.push(header_names[col_idx].clone());
                } else {
                    let trimmed = field.trim();
                    type_stats[col_idx].total_non_missing += 1;

                    let mut is_num = false;
                    if is_int_val(trimmed) {
                        type_stats[col_idx].int_count += 1;
                        is_num = true;
                    } else if is_float_val(trimmed) {
                        type_stats[col_idx].float_count += 1;
                        is_num = true;
                    } else if is_bool_val(trimmed) {
                        type_stats[col_idx].bool_count += 1;
                    } else if is_date_val(trimmed) {
                        type_stats[col_idx].date_count += 1;
                    }

                    if is_num {
                        if let Ok(num) = trimmed.parse::<f64>() {
                            numeric_values[col_idx].push(num);
                        }
                    }
                }
            } else if is_missing {
                row_has_missing = true;
                missing_cols_in_row.push(format!("col_{}", col_idx + 1));
            }
        }

        if record.len() < col_count {
            row_has_missing = true;
            for col_idx in record.len()..col_count {
                missing_counts[col_idx] += 1;
                missing_cols_in_row.push(header_names[col_idx].clone());
            }
        }

        if row_has_missing {
            rows_with_missing += 1;
            if missing_row_samples.len() < 10 || cli.verbose {
                missing_row_samples.push((row_idx + 1, missing_cols_in_row));
            }
        }

        // 2. Duplicate checking collection
        if check_duplicates || is_repair_mode {
            if let Some(col_idx) = key_col_idx {
                if col_idx < record.len() {
                    let val = record[col_idx].trim().to_string();
                    if !is_missing_value(&val) {
                        let entry = key_occurrences.entry(val).or_default();
                        entry.push(row_idx + 1);
                        if entry.len() > 1 {
                            duplicate_row_indices.insert(row_idx + 1);
                        }
                    }
                }
            } else {
                let h = hash_record(&record);
                let fields: Vec<String> = record.iter().map(|s| s.to_string()).collect();
                let bucket = row_hashes.entry(h).or_default();
                bucket.push((row_idx + 1, fields));
            }
        }
    }

    // Infer dominant types for each column
    let inferred_types: Vec<DataType> = type_stats.iter().map(infer_type).collect();

    // Compute outlier bounds and medians for numeric columns
    let mut outlier_bounds: Vec<Option<OutlierBounds>> = Vec::with_capacity(col_count);
    let mut col_medians: Vec<Option<f64>> = Vec::with_capacity(col_count);
    for col_idx in 0..col_count {
        if inferred_types[col_idx].is_numeric() {
            let mut vals = numeric_values[col_idx].clone();
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            outlier_bounds.push(compute_outlier_bounds(&vals));
            col_medians.push(calculate_median(&vals));
        } else {
            outlier_bounds.push(None);
            col_medians.push(None);
        }
    }

    // For full-row duplicates, mark duplicates (indices > 1)
    if key_col_idx.is_none() && (check_duplicates || is_repair_mode) {
        for items in row_hashes.values() {
            if items.len() > 1 {
                let mut seen_fields: Vec<Vec<String>> = Vec::new();
                for (row_no, fields) in items {
                    if seen_fields.contains(fields) {
                        duplicate_row_indices.insert(*row_no);
                    } else {
                        seen_fields.push(fields.clone());
                    }
                }
            }
        }
    }

    // Pass 2: Type mismatches & Outlier detection
    let mut mismatch_counts = vec![0usize; col_count];
    let mut mismatch_details: Vec<(usize, String, &'static str, String, &'static str)> =
        Vec::new();

    let mut outlier_counts = vec![0usize; col_count];
    let mut outlier_details: Vec<(usize, String, f64, String, String, f64, f64)> = Vec::new();

    if check_types || check_outliers {
        if let Err(err) = file.seek(std::io::SeekFrom::Start(0)) {
            eprintln!("Error rewinding file for validation: {}", err);
            return;
        }

        let mut rdr2 = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(&file);

        for (row_idx, result) in rdr2.records().enumerate() {
            let record = match result {
                Ok(rec) => rec,
                Err(_) => continue,
            };

            for (col_idx, field) in record.iter().enumerate() {
                if col_idx < col_count {
                    if is_missing_value(field) {
                        continue;
                    }

                    let expected = inferred_types[col_idx];

                    // Check type mismatch
                    if check_types {
                        if let Err(reason) = validate_type_match(field, expected) {
                            mismatch_counts[col_idx] += 1;
                            if mismatch_details.len() < 10 || cli.verbose {
                                mismatch_details.push((
                                    row_idx + 1,
                                    header_names[col_idx].clone(),
                                    expected.name(),
                                    field.to_string(),
                                    reason,
                                ));
                            }
                        }
                    }

                    // Check outlier (only if column is numeric and value parses as number)
                    if check_outliers && expected.is_numeric() {
                        if let Some(bounds) = &outlier_bounds[col_idx] {
                            if let Ok(val) = field.trim().parse::<f64>() {
                                let is_outlier =
                                    val < bounds.lower_bound || val > bounds.upper_bound;
                                if is_outlier {
                                    outlier_counts[col_idx] += 1;
                                    let reason = if val > bounds.upper_bound {
                                        if bounds.iqr > 0.0 {
                                            format!(
                                                "Exceeds upper bound {} (+{:.1}x IQR)",
                                                format_num(bounds.upper_bound),
                                                (val - bounds.q3) / bounds.iqr
                                            )
                                        } else {
                                            format!(
                                                "Exceeds identical bound {}",
                                                format_num(bounds.upper_bound)
                                            )
                                        }
                                    } else {
                                        if bounds.iqr > 0.0 {
                                            format!(
                                                "Below lower bound {} (-{:.1}x IQR)",
                                                format_num(bounds.lower_bound),
                                                (bounds.q1 - val) / bounds.iqr
                                            )
                                        } else {
                                            format!(
                                                "Below identical bound {}",
                                                format_num(bounds.lower_bound)
                                            )
                                        }
                                    };

                                    if outlier_details.len() < 10 || cli.verbose {
                                        let range_str = format!(
                                            "[{}, {}]",
                                            format_num(bounds.lower_bound),
                                            format_num(bounds.upper_bound)
                                        );
                                        outlier_details.push((
                                            row_idx + 1,
                                            header_names[col_idx].clone(),
                                            val,
                                            range_str,
                                            reason,
                                            bounds.lower_bound,
                                            bounds.upper_bound,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Duplicate detection processing for display
    let mut duplicate_groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut total_duplicate_rows = 0usize;

    if check_duplicates {
        if let Some(col_idx) = key_col_idx {
            let key_name = &header_names[col_idx];
            for (key_val, rows) in key_occurrences {
                if rows.len() > 1 {
                    total_duplicate_rows += rows.len() - 1;
                    duplicate_groups.push((
                        format!("Key '{}' = \"{}\"", key_name, key_val),
                        rows,
                    ));
                }
            }
        } else {
            for (_hash, items) in row_hashes {
                if items.len() > 1 {
                    let mut clusters: Vec<Vec<usize>> = Vec::new();
                    let mut cluster_fields: Vec<Vec<String>> = Vec::new();

                    for (row_no, fields) in items {
                        let mut found_cluster = false;
                        for (c_idx, c_fields) in cluster_fields.iter().enumerate() {
                            if *c_fields == fields {
                                clusters[c_idx].push(row_no);
                                found_cluster = true;
                                break;
                            }
                        }
                        if !found_cluster {
                            cluster_fields.push(fields);
                            clusters.push(vec![row_no]);
                        }
                    }

                    for cluster in clusters {
                        if cluster.len() > 1 {
                            total_duplicate_rows += cluster.len() - 1;
                            duplicate_groups.push(("Identical full row".to_string(), cluster));
                        }
                    }
                }
            }
        }
        duplicate_groups.sort_by_key(|(_, rows)| rows.first().copied().unwrap_or(0));
    }

    // ---------------- REPAIR / PATCH EXECUTION ----------------
    let mut repair_report_data: Option<RepairReportJson> = None;
    let mut patch_report_data: Option<PatchReportJson> = None;

    if is_repair_mode {
        let (out_path, is_temp_in_place) = if cli.in_place {
            let mut temp = target_path.clone();
            temp.set_extension("tmp_handler_edit");
            (temp, true)
        } else if let Some(ref p) = cli.output {
            (p.clone(), false)
        } else {
            let mut p = target_path.clone();
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("data");
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("csv");
            let suffix = if has_patches && !cli.fix { "patched" } else { "clean" };
            p.set_file_name(format!("{}_{}.{}", stem, suffix, ext));
            (p, false)
        };

        if let Err(err) = file.seek(std::io::SeekFrom::Start(0)) {
            eprintln!("Error rewinding file for repair: {}", err);
            return;
        }

        let mut rdr_repair = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(&file);

        let mut writer = match csv::Writer::from_path(&out_path) {
            Ok(w) => w,
            Err(err) => {
                eprintln!("Error creating output file '{}': {}", out_path.display(), err);
                process::exit(1);
            }
        };

        // Write header
        if let Err(err) = writer.write_record(&headers) {
            eprintln!("Error writing CSV header: {}", err);
            process::exit(1);
        }

        let mut output_rows = 0usize;
        let mut duplicates_dropped = 0usize;
        let mut invalid_rows_dropped = 0usize;
        let mut outliers_dropped = 0usize;
        let mut missing_rows_dropped = 0usize;
        let mut cells_imputed = 0usize;
        let mut types_coerced = 0usize;
        let mut patches_applied_list: Vec<PatchAppliedJson> = Vec::new();
        let mut pinpoint_rows_dropped_list: Vec<usize> = Vec::new();

        for (row_idx, result) in rdr_repair.records().enumerate() {
            let record = match result {
                Ok(rec) => rec,
                Err(_) => continue,
            };

            let row_no = row_idx + 1;

            // Pinpoint drop check
            if pinpoint_drop_rows.contains(&row_no) {
                pinpoint_rows_dropped_list.push(row_no);
                continue;
            }

            // Duplicate row drop check
            if do_drop_dups && duplicate_row_indices.contains(&row_no) {
                duplicates_dropped += 1;
                continue;
            }

            let mut clean_fields: Vec<String> = Vec::with_capacity(col_count);
            let mut drop_this_row = false;

            for (col_idx, field) in record.iter().enumerate() {
                if col_idx >= col_count {
                    break;
                }

                // Check if this cell has a surgical patch
                let raw_val = if let Some(patched_val) = surgical_patches.get(&(row_no, col_idx)) {
                    patches_applied_list.push(PatchAppliedJson {
                        row: row_no,
                        column: header_names[col_idx].clone(),
                        old_value: field.to_string(),
                        new_value: patched_val.clone(),
                    });
                    patched_val.as_str()
                } else {
                    field
                };

                let is_missing = is_missing_value(raw_val);
                let expected = inferred_types[col_idx];

                if is_missing {
                    if do_drop_missing {
                        missing_rows_dropped += 1;
                        drop_this_row = true;
                        break;
                    } else if do_fill_missing {
                        if expected.is_numeric() {
                            if let Some(med) = col_medians[col_idx] {
                                clean_fields.push(format_num(med));
                                cells_imputed += 1;
                                continue;
                            }
                        }
                        clean_fields.push(String::new());
                    } else {
                        clean_fields.push(String::new());
                    }
                } else {
                    let trimmed = raw_val.trim();

                    // Check Outliers
                    if do_drop_outliers && expected.is_numeric() {
                        if let Some(bounds) = &outlier_bounds[col_idx] {
                            if let Ok(num) = trimmed.parse::<f64>() {
                                if num < bounds.lower_bound || num > bounds.upper_bound {
                                    outliers_dropped += 1;
                                    drop_this_row = true;
                                    break;
                                }
                            }
                        }
                    }

                    // Check Type validation and coercion
                    match expected {
                        DataType::Integer => {
                            if is_int_val(trimmed) {
                                clean_fields.push(trimmed.to_string());
                            } else if do_coerce {
                                if let Some(coerced) = coerce_int(trimmed) {
                                    clean_fields.push(coerced);
                                    types_coerced += 1;
                                } else if do_drop_invalid {
                                    invalid_rows_dropped += 1;
                                    drop_this_row = true;
                                    break;
                                } else {
                                    clean_fields.push(trimmed.to_string());
                                }
                            } else if do_drop_invalid {
                                invalid_rows_dropped += 1;
                                drop_this_row = true;
                                break;
                            } else {
                                clean_fields.push(trimmed.to_string());
                            }
                        }
                        DataType::Float => {
                            if trimmed.parse::<f64>().is_ok() {
                                clean_fields.push(trimmed.to_string());
                            } else if do_drop_invalid {
                                invalid_rows_dropped += 1;
                                drop_this_row = true;
                                break;
                            } else {
                                clean_fields.push(trimmed.to_string());
                            }
                        }
                        DataType::Boolean => {
                            if let Some(b) = coerce_bool(trimmed) {
                                if b != trimmed {
                                    types_coerced += 1;
                                }
                                clean_fields.push(b.to_string());
                            } else if do_drop_invalid {
                                invalid_rows_dropped += 1;
                                drop_this_row = true;
                                break;
                            } else {
                                clean_fields.push(trimmed.to_string());
                            }
                        }
                        DataType::Date => {
                            if is_date_val(trimmed) {
                                clean_fields.push(trimmed.to_string());
                            } else if do_drop_invalid {
                                invalid_rows_dropped += 1;
                                drop_this_row = true;
                                break;
                            } else {
                                clean_fields.push(trimmed.to_string());
                            }
                        }
                        DataType::Text => {
                            clean_fields.push(trimmed.to_string());
                        }
                    }
                }
            }

            // Fill missing trailing columns
            while clean_fields.len() < col_count && !drop_this_row {
                clean_fields.push(String::new());
            }

            if !drop_this_row {
                if let Err(err) = writer.write_record(&clean_fields) {
                    eprintln!("Warning: Error writing clean record: {}", err);
                } else {
                    output_rows += 1;
                }
            }
        }

        let _ = writer.flush();

        // If in-place modification requested, atomically rename temp file
        let final_destination = if is_temp_in_place {
            if let Err(err) = std::fs::rename(&out_path, &target_path) {
                eprintln!("Error replacing original file in-place: {}", err);
                process::exit(1);
            }
            target_path.clone()
        } else {
            out_path.clone()
        };

        let total_dropped = duplicates_dropped
            + invalid_rows_dropped
            + outliers_dropped
            + missing_rows_dropped
            + pinpoint_rows_dropped_list.len();

        if cli.fix || cli.drop_duplicates || cli.drop_invalid || cli.drop_outliers || cli.drop_missing || cli.fill_missing || cli.coerce_types {
            repair_report_data = Some(RepairReportJson {
                input_file: target_path.display().to_string(),
                output_file: final_destination.display().to_string(),
                input_rows: total_rows,
                output_rows,
                total_rows_dropped: total_dropped,
                duplicates_dropped,
                invalid_rows_dropped,
                outliers_dropped,
                missing_rows_dropped,
                cells_imputed,
                types_coerced,
                status: "success".to_string(),
            });
        }

        if has_patches {
            patch_report_data = Some(PatchReportJson {
                input_file: target_path.display().to_string(),
                output_file: final_destination.display().to_string(),
                patches_applied: patches_applied_list.clone(),
                rows_dropped: pinpoint_rows_dropped_list.clone(),
                status: "success".to_string(),
            });
        }

        if !cli.json {
            if has_patches {
                println!("\n🩹 Pinpoint Patch & Surgical Edit Report:");
                let mut patch_table = Table::new();
                patch_table
                    .load_preset(UTF8_FULL)
                    .apply_modifier(UTF8_ROUND_CORNERS)
                    .set_header(vec!["Row #", "Column", "Old Value", "New Value", "Status"]);

                for p in &patches_applied_list {
                    patch_table.add_row(vec![
                        Cell::new(p.row.to_string()),
                        Cell::new(&p.column),
                        Cell::new(format!("\"{}\"", p.old_value)),
                        Cell::new(format!("\"{}\"", p.new_value)).fg(Color::Green),
                        Cell::new("✔ Patched").fg(Color::Green),
                    ]);
                }

                if !pinpoint_rows_dropped_list.is_empty() {
                    for r in &pinpoint_rows_dropped_list {
                        patch_table.add_row(vec![
                            Cell::new(r.to_string()),
                            Cell::new("-"),
                            Cell::new("-"),
                            Cell::new("-"),
                            Cell::new("✖ Row Dropped").fg(Color::Yellow),
                        ]);
                    }
                }

                println!("{}", patch_table);
                println!("Saved to: {}\n", final_destination.display());
            }

            if repair_report_data.is_some() {
                println!("\n🛠️ CSV Repair Report:");
                let mut rep_table = Table::new();
                rep_table
                    .load_preset(UTF8_FULL)
                    .apply_modifier(UTF8_ROUND_CORNERS)
                    .set_header(vec!["Repair Metric", "Value"]);

                rep_table.add_row(vec![
                    Cell::new("Input File"),
                    Cell::new(format!("{} ({} rows)", target_path.display(), total_rows)),
                ]);
                rep_table.add_row(vec![
                    Cell::new("Output Clean File"),
                    Cell::new(format!("{} ({} rows)", final_destination.display(), output_rows)),
                ]);
                rep_table.add_row(vec![
                    Cell::new("Total Rows Dropped"),
                    Cell::new(format!(
                        "{} ({:.1}%)",
                        total_dropped,
                        if total_rows > 0 {
                            (total_dropped as f64 / total_rows as f64) * 100.0
                        } else {
                            0.0
                        }
                    ))
                    .fg(if total_dropped > 0 { Color::Yellow } else { Color::Green }),
                ]);
                rep_table.add_row(vec![
                    Cell::new("  - Duplicate Rows Dropped"),
                    Cell::new(duplicates_dropped.to_string()),
                ]);
                rep_table.add_row(vec![
                    Cell::new("  - Invalid Type Rows Dropped"),
                    Cell::new(invalid_rows_dropped.to_string()),
                ]);
                rep_table.add_row(vec![
                    Cell::new("  - Outlier Rows Dropped"),
                    Cell::new(outliers_dropped.to_string()),
                ]);
                rep_table.add_row(vec![
                    Cell::new("  - Missing Value Rows Dropped"),
                    Cell::new(missing_rows_dropped.to_string()),
                ]);
                if !pinpoint_rows_dropped_list.is_empty() {
                    rep_table.add_row(vec![
                        Cell::new("  - Pinpoint Rows Dropped"),
                        Cell::new(pinpoint_rows_dropped_list.len().to_string()),
                    ]);
                }
                rep_table.add_row(vec![
                    Cell::new("Cells Imputed (Filled)"),
                    Cell::new(cells_imputed.to_string()),
                ]);
                rep_table.add_row(vec![
                    Cell::new("Values Coerced (Standardized)"),
                    Cell::new(types_coerced.to_string()),
                ]);
                rep_table.add_row(vec![
                    Cell::new("Export Status"),
                    Cell::new("✔ Clean CSV Exported Successfully").fg(Color::Green),
                ]);

                println!("{}", rep_table);
            }
        }
    }

    // ---------------- IF JSON OUTPUT REQUESTED ----------------
    if cli.json {
        let missing_json = if check_missing {
            let total_missing: usize = missing_counts.iter().sum();
            let missing_pct = if total_rows > 0 {
                (rows_with_missing as f64 / total_rows as f64) * 100.0
            } else {
                0.0
            };

            let cols: Vec<MissingColJson> = header_names
                .iter()
                .zip(missing_counts.iter())
                .enumerate()
                .map(|(i, (name, &count))| {
                    let pct = if total_rows > 0 {
                        (count as f64 / total_rows as f64) * 100.0
                    } else {
                        0.0
                    };
                    MissingColJson {
                        index: i + 1,
                        name: name.clone(),
                        missing_count: count,
                        missing_percent: (pct * 10.0).round() / 10.0,
                        is_clean: count == 0,
                    }
                })
                .collect();

            let affected: Vec<MissingRowSampleJson> = missing_row_samples
                .iter()
                .map(|(row, cols)| MissingRowSampleJson {
                    row: *row,
                    missing_columns: cols.clone(),
                })
                .collect();

            Some(MissingReportJson {
                total_missing_values: total_missing,
                rows_with_missing,
                rows_with_missing_pct: (missing_pct * 100.0).round() / 100.0,
                columns: cols,
                affected_rows: affected,
            })
        } else {
            None
        };

        let type_json = if check_types {
            let total_mismatches: usize = mismatch_counts.iter().sum();
            let cols: Vec<TypeColJson> = header_names
                .iter()
                .zip(inferred_types.iter())
                .zip(mismatch_counts.iter())
                .zip(type_stats.iter())
                .enumerate()
                .map(|(i, (((name, &expected), &mismatches), stats))| TypeColJson {
                    index: i + 1,
                    name: name.clone(),
                    inferred_type: expected.name().to_string(),
                    valid_cells: stats.total_non_missing,
                    mismatches,
                    is_clean: mismatches == 0,
                })
                .collect();

            let details: Vec<TypeMismatchDetailJson> = mismatch_details
                .iter()
                .map(|(row, col, expected, found, reason)| TypeMismatchDetailJson {
                    row: *row,
                    column: col.clone(),
                    expected_type: expected.to_string(),
                    found_value: found.clone(),
                    reason: reason.to_string(),
                })
                .collect();

            Some(TypeReportJson {
                total_mismatches,
                columns: cols,
                mismatch_details: details,
            })
        } else {
            None
        };

        let outlier_json = if check_outliers {
            let total_outliers: usize = outlier_counts.iter().sum();
            let cols: Vec<OutlierColJson> = header_names
                .iter()
                .zip(inferred_types.iter())
                .zip(outlier_bounds.iter())
                .zip(outlier_counts.iter())
                .enumerate()
                .map(|(i, (((name, &expected), bounds_opt), &count))| {
                    let is_num = expected.is_numeric();
                    let (lb, ub, mn, mx) = match bounds_opt {
                        Some(b) => (Some(b.lower_bound), Some(b.upper_bound), Some(b.min), Some(b.max)),
                        None => (None, None, None, None),
                    };
                    OutlierColJson {
                        index: i + 1,
                        name: name.clone(),
                        inferred_type: expected.name().to_string(),
                        is_numeric: is_num,
                        lower_bound: lb,
                        upper_bound: ub,
                        min: mn,
                        max: mx,
                        outliers: count,
                        is_clean: count == 0 && is_num,
                    }
                })
                .collect();

            let details: Vec<OutlierDetailJson> = outlier_details
                .iter()
                .map(|(row, col, val, _range_str, reason, lb, ub)| OutlierDetailJson {
                    row: *row,
                    column: col.clone(),
                    value: *val,
                    lower_bound: *lb,
                    upper_bound: *ub,
                    reason: reason.clone(),
                })
                .collect();

            Some(OutlierReportJson {
                method: "IQR".to_string(),
                total_outliers,
                columns: cols,
                outlier_details: details,
            })
        } else {
            None
        };

        let duplicate_json = if check_duplicates {
            let scope_str = match &cli.key {
                Some(k) => format!("Key Column '{}'", k),
                None => "Full Row".to_string(),
            };

            let pct = if total_rows > 0 {
                (total_duplicate_rows as f64 / total_rows as f64) * 100.0
            } else {
                0.0
            };

            let clusters: Vec<DuplicateClusterJson> = duplicate_groups
                .iter()
                .enumerate()
                .map(|(i, (desc, rows))| DuplicateClusterJson {
                    cluster_index: i + 1,
                    description: desc.clone(),
                    occurrences: rows.len(),
                    row_numbers: rows.clone(),
                })
                .collect();

            Some(DuplicateReportJson {
                scope: scope_str,
                total_rows,
                unique_records: total_rows - total_duplicate_rows,
                duplicate_rows: total_duplicate_rows,
                duplicate_percent: (pct * 10.0).round() / 10.0,
                clusters,
            })
        } else {
            None
        };

        let report = FullReportJson {
            file_path: target_path.display().to_string(),
            total_rows,
            total_columns: col_count,
            headers: header_names,
            missing_values: missing_json,
            type_validation: type_json,
            outliers: outlier_json,
            duplicates: duplicate_json,
            repair: repair_report_data,
            surgical_patch: patch_report_data,
        };

        match serde_json::to_string_pretty(&report) {
            Ok(json_str) => println!("{}", json_str),
            Err(err) => eprintln!("Error formatting JSON: {}", err),
        }
        return;
    }

    // ---------------- TEXT / TERMINAL TABLES OUTPUT ----------------
    if check_missing || check_types || check_outliers || check_duplicates {
        println!("\n🔍 Analyzing '{}'...\n", target_path.display());
    }

    // ---------------- MISSING VALUES REPORT ----------------
    if check_missing {
        let total_missing: usize = missing_counts.iter().sum();

        let mut summary_table = Table::new();
        summary_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Missing Values Metric", "Value"]);

        summary_table.add_row(vec![
            Cell::new("File Path"),
            Cell::new(target_path.display().to_string()),
        ]);
        summary_table.add_row(vec![
            Cell::new("Total Rows"),
            Cell::new(total_rows.to_string()),
        ]);
        summary_table.add_row(vec![
            Cell::new("Total Columns"),
            Cell::new(col_count.to_string()),
        ]);
        summary_table.add_row(vec![
            Cell::new("Total Missing Values"),
            Cell::new(total_missing.to_string()).fg(if total_missing > 0 {
                Color::Red
            } else {
                Color::Green
            }),
        ]);
        summary_table.add_row(vec![
            Cell::new("Rows with Missing Values"),
            Cell::new(format!(
                "{} ({:.2}%)",
                rows_with_missing,
                if total_rows > 0 {
                    (rows_with_missing as f64 / total_rows as f64) * 100.0
                } else {
                    0.0
                }
            )),
        ]);

        println!("{}", summary_table);

        println!("\n📋 Missing Values by Column:");
        let mut col_table = Table::new();
        col_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                "Index",
                "Column Name",
                "Missing Count",
                "Missing %",
                "Status",
            ]);

        for (i, (name, &count)) in header_names.iter().zip(missing_counts.iter()).enumerate() {
            let pct = if total_rows > 0 {
                (count as f64 / total_rows as f64) * 100.0
            } else {
                0.0
            };

            let status = if count == 0 {
                Cell::new("✔ Clean").fg(Color::Green)
            } else {
                Cell::new(format!("✖ {} missing", count)).fg(Color::Red)
            };

            col_table.add_row(vec![
                Cell::new((i + 1).to_string()),
                Cell::new(name),
                Cell::new(count.to_string()),
                Cell::new(format!("{:.1}%", pct)),
                status,
            ]);
        }

        println!("{}", col_table);

        if !missing_row_samples.is_empty() {
            println!("\n⚠️ Rows with Missing Values:");
            let sample_limit = if cli.verbose {
                missing_row_samples.len()
            } else {
                missing_row_samples.len().min(10)
            };

            let mut row_table = Table::new();
            row_table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec!["Row #", "Missing Columns"]);

            for (row_no, missing_cols) in &missing_row_samples[..sample_limit] {
                row_table.add_row(vec![
                    Cell::new(row_no.to_string()),
                    Cell::new(missing_cols.join(", ")),
                ]);
            }

            println!("{}", row_table);

            if !cli.verbose && rows_with_missing > 10 {
                println!(
                    "💡 Showing first 10 of {} affected rows. Use --verbose to see all rows.",
                    rows_with_missing
                );
            }
        } else {
            println!("\n🎉 No missing values detected!");
        }
    }

    // ---------------- TYPE MISMATCH REPORT ----------------
    if check_types {
        let total_mismatches: usize = mismatch_counts.iter().sum();

        println!("\n🏷️ Type Mismatch Validation Report:");

        let mut type_table = Table::new();
        type_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                "Index",
                "Column Name",
                "Inferred Type",
                "Valid Cells",
                "Mismatches",
                "Status",
            ]);

        for (i, (((name, &expected), &mismatches), stats)) in header_names
            .iter()
            .zip(inferred_types.iter())
            .zip(mismatch_counts.iter())
            .zip(type_stats.iter())
            .enumerate()
        {
            let status = if mismatches == 0 {
                Cell::new("✔ Clean").fg(Color::Green)
            } else {
                Cell::new(format!("✖ {} mismatch(es)", mismatches)).fg(Color::Red)
            };

            type_table.add_row(vec![
                Cell::new((i + 1).to_string()),
                Cell::new(name),
                Cell::new(expected.name()),
                Cell::new(stats.total_non_missing.to_string()),
                Cell::new(mismatches.to_string()),
                status,
            ]);
        }

        println!("{}", type_table);

        if !mismatch_details.is_empty() {
            println!("\n⚠️ Type Mismatch Details:");
            let detail_limit = if cli.verbose {
                mismatch_details.len()
            } else {
                mismatch_details.len().min(10)
            };

            let mut detail_table = Table::new();
            detail_table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec![
                    "Row #",
                    "Column",
                    "Expected Type",
                    "Found Value",
                    "Reason",
                ]);

            for (row_no, col_name, expected_type, found_val, reason) in
                &mismatch_details[..detail_limit]
            {
                detail_table.add_row(vec![
                    Cell::new(row_no.to_string()),
                    Cell::new(col_name),
                    Cell::new(expected_type),
                    Cell::new(format!("\"{}\"", found_val)),
                    Cell::new(reason),
                ]);
            }

            println!("{}", detail_table);

            if !cli.verbose && total_mismatches > 10 {
                println!(
                    "💡 Showing first 10 of {} type mismatches. Use --verbose to see all details.",
                    total_mismatches
                );
            }
        } else {
            println!("\n🎉 No type mismatches detected!");
        }
    }

    // ---------------- OUTLIER DETECTION REPORT ----------------
    if check_outliers {
        let total_outliers: usize = outlier_counts.iter().sum();

        println!("\n📈 Outlier Detection Report (Method: IQR):");

        let mut outlier_table = Table::new();
        outlier_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                "Index",
                "Column Name",
                "Inferred Type",
                "IQR Expected Range",
                "Min / Max",
                "Outliers",
                "Status",
            ]);

        for (i, ((name, &expected), bounds_opt)) in header_names
            .iter()
            .zip(inferred_types.iter())
            .zip(outlier_bounds.iter())
            .enumerate()
        {
            if !expected.is_numeric() {
                outlier_table.add_row(vec![
                    Cell::new((i + 1).to_string()),
                    Cell::new(name),
                    Cell::new(expected.name()),
                    Cell::new("-"),
                    Cell::new("-"),
                    Cell::new("0"),
                    Cell::new("ℹ Non-numeric"),
                ]);
                continue;
            }

            match bounds_opt {
                Some(b) => {
                    let count = outlier_counts[i];
                    let status = if count == 0 {
                        Cell::new("✔ Clean").fg(Color::Green)
                    } else {
                        Cell::new(format!("✖ {} outlier(s)", count)).fg(Color::Red)
                    };

                    outlier_table.add_row(vec![
                        Cell::new((i + 1).to_string()),
                        Cell::new(name),
                        Cell::new(expected.name()),
                        Cell::new(format!(
                            "[{}, {}]",
                            format_num(b.lower_bound),
                            format_num(b.upper_bound)
                        )),
                        Cell::new(format!("{} / {}", format_num(b.min), format_num(b.max))),
                        Cell::new(count.to_string()),
                        status,
                    ]);
                }
                None => {
                    outlier_table.add_row(vec![
                        Cell::new((i + 1).to_string()),
                        Cell::new(name),
                        Cell::new(expected.name()),
                        Cell::new("-"),
                        Cell::new("-"),
                        Cell::new("0"),
                        Cell::new("ℹ Too few data points (< 4)"),
                    ]);
                }
            }
        }

        println!("{}", outlier_table);

        if !outlier_details.is_empty() {
            println!("\n⚠️ Outlier Details:");
            let detail_limit = if cli.verbose {
                outlier_details.len()
            } else {
                outlier_details.len().min(10)
            };

            let mut detail_table = Table::new();
            detail_table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec![
                    "Row #",
                    "Column",
                    "Value",
                    "IQR Valid Range",
                    "Reason",
                ]);

            for (row_no, col_name, val, range_str, reason, _lb, _ub) in
                &outlier_details[..detail_limit]
            {
                detail_table.add_row(vec![
                    Cell::new(row_no.to_string()),
                    Cell::new(col_name),
                    Cell::new(format_num(*val)),
                    Cell::new(range_str),
                    Cell::new(reason),
                ]);
            }

            println!("{}", detail_table);

            if !cli.verbose && total_outliers > 10 {
                println!(
                    "💡 Showing first 10 of {} outliers. Use --verbose to see all details.",
                    total_outliers
                );
            }
        } else {
            println!("\n🎉 No numerical outliers detected!");
        }
    }

    // ---------------- DUPLICATE DETECTION REPORT ----------------
    if check_duplicates {
        println!("\n🔁 Duplicate Detection Report:");

        let scope_str = match &cli.key {
            Some(k) => format!("Key Column '{}'", k),
            None => "Full Row (All columns identical)".to_string(),
        };

        let mut dup_summary = Table::new();
        dup_summary
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Duplicate Metric", "Value"]);

        dup_summary.add_row(vec![Cell::new("Scope"), Cell::new(scope_str)]);
        dup_summary.add_row(vec![
            Cell::new("Total Rows"),
            Cell::new(total_rows.to_string()),
        ]);
        dup_summary.add_row(vec![
            Cell::new("Unique Records"),
            Cell::new((total_rows - total_duplicate_rows).to_string()),
        ]);
        dup_summary.add_row(vec![
            Cell::new("Duplicate Rows"),
            Cell::new(format!(
                "{} ({:.2}%)",
                total_duplicate_rows,
                if total_rows > 0 {
                    (total_duplicate_rows as f64 / total_rows as f64) * 100.0
                } else {
                    0.0
                }
            ))
            .fg(if total_duplicate_rows > 0 {
                Color::Red
            } else {
                Color::Green
            }),
        ]);
        dup_summary.add_row(vec![
            Cell::new("Duplicate Clusters"),
            Cell::new(duplicate_groups.len().to_string()),
        ]);

        println!("{}", dup_summary);

        if !duplicate_groups.is_empty() {
            println!("\n⚠️ Duplicate Row Clusters:");

            let limit = if cli.verbose {
                duplicate_groups.len()
            } else {
                duplicate_groups.len().min(10)
            };

            let mut dup_table = Table::new();
            dup_table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec![
                    "Cluster #",
                    "Description",
                    "Occurrences",
                    "Row Numbers",
                ]);

            for (idx, (desc, rows)) in duplicate_groups[..limit].iter().enumerate() {
                let row_str = rows
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");

                dup_table.add_row(vec![
                    Cell::new((idx + 1).to_string()),
                    Cell::new(desc),
                    Cell::new(format!("{} rows", rows.len())),
                    Cell::new(format!("Rows {}", row_str)),
                ]);
            }

            println!("{}", dup_table);

            if !cli.verbose && duplicate_groups.len() > 10 {
                println!(
                    "💡 Showing first 10 of {} duplicate clusters. Use --verbose to see all.",
                    duplicate_groups.len()
                );
            }
        } else {
            println!("\n🎉 No duplicate rows detected!");
        }
    }
}

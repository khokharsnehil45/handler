# handler ⚡

[![Rust](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-lightgrey.svg)]()
[![Handbook PDF](https://img.shields.io/badge/Documentation-Handbook%20PDF-purple.svg)](docs/HANDLER_USER_GUIDE.pdf)

> A fast, memory-efficient CLI tool and autonomous agent engine built with Rust for **auditing**, **surgically patching**, and **automatically repairing** CSV files.

📘 **Read the Comprehensive Guide**: [**HANDLER_USER_GUIDE.pdf**](docs/HANDLER_USER_GUIDE.pdf) *(14-page guide from zero knowledge to advanced AI agent integration)*

---

## 🚀 Quick Install (Linux / macOS)

Install the latest release with a single `curl` command:

```bash
curl -sSL https://raw.githubusercontent.com/khokharsnehil45/handler/main/install.sh | bash
```

*Or install from source with Cargo:*
```bash
cargo install --git https://github.com/khokharsnehil45/handler.git
```

---

## 🌟 Why `handler`?

Dirty CSV files break machine learning pipelines, ETL jobs, and AI agents. Most tools either only **diagnose** (without repairing) or blindly **drop rows** (destroying data).

`handler` delivers the complete lifecycle in a compiled, native binary:
1. **Missing Value Detection**: Detects blank strings, whitespace-only cells, and messy null markers (`NA`, `null`, `None`, `-`, `?`).
2. **Automatic Type Inference & Mismatch Auditing**: Discovers column types (`Integer`, `Float`, `Boolean`, `Date`, `Text`) and pinpoints invalid entries.
3. **Statistical Outlier Detection (IQR)**: Computes 25th percentile ($Q_1$), 75th percentile ($Q_3$), and flags anomalies outside $1.5 \times IQR$.
4. **Duplicate & Collision Detection**: Scans for exact full-row duplicates or primary key collisions (`--key <COL>`).
5. **Surgical Pinpoint Patching (`--patch` / `--drop-row`)**: Surgically repair individual cells without dropping whole rows.
6. **Bulk Auto-Remediation (`--fix`)**: Clean, coerce, and sanitize dirty datasets into a new file.
7. **Agent-Native JSON (`--json`)**: Machine-readable JSON payloads designed for autonomous LLM workflows.

---

## 📖 Usage Guide

### 1. Full Audit (Terminal Tables)
Run a complete health check on any CSV file:
```bash
handler data.csv
# or:
handler --file_path data.csv --audit
```

### 2. Specific Diagnostic Checks
```bash
# Check only for missing values:
handler data.csv --search_missing

# Check only for type mismatches:
handler data.csv --search_types

# Check only for numerical outliers:
handler data.csv --search_outliers

# Check for duplicate rows:
handler data.csv --search_duplicates

# Check for duplicates on a specific primary key:
handler data.csv --key user_id
```

### 3. Machine-Readable JSON Mode (For AI Agents)
```bash
handler data.csv --json
```

---

## 🩹 Surgical Pinpoint Patching

When you know the exact cells to fix, patch them with surgical precision:

```bash
# Patch cell by column name:
handler data.csv --patch "3:age=22" --output clean.csv

# Patch cell by column index (1-based):
handler data.csv --patch "3:3=22" --output clean.csv

# Patch multiple cells simultaneously:
handler data.csv --patch "3:age=22" --patch "5:signup_date=2026-03-01" --output clean.csv

# Drop a specific corrupted row:
handler data.csv --drop-row 10 --output clean.csv

# Apply edits in-place directly to the original file:
handler data.csv --patch "3:age=22" --in-place
```

---

## 🛠️ Bulk Auto-Repair & Remediation

```bash
# Smart auto-repair (saves to <input>_clean.csv):
handler data.csv --fix

# Specify custom output path:
handler data.csv --fix --output sanitized.csv

# Impute missing numeric cells with column medians:
handler data.csv --fix --fill-missing --output sanitized.csv

# Drop rows containing statistical outliers:
handler data.csv --fix --drop-outliers --output sanitized.csv

# Drop rows with any missing values:
handler data.csv --fix --drop-missing --output sanitized.csv

# Export JSON repair summary for agents:
handler data.csv --fix --output sanitized.csv --json
```

---

## 📋 Command-Line Reference

| Flag / Option | Description |
|---|---|
| `-f, --file, --file_path <PATH>` | Path to the target CSV file |
| `-a, --audit` | Run full health audit (Missing, Types, Outliers, Duplicates) |
| `-s, --search_missing` | Search for missing values |
| `-t, --search_types` | Search for type mismatches |
| `-o, --search_outliers` | Search for statistical outliers using IQR |
| `-d, --search_duplicates` | Search for duplicate rows |
| `-k, --key <COLUMN>` | Check for duplicate records on a specific unique key |
| `-j, --json` | Output machine-readable JSON for agents |
| `-v, --verbose` | Show all affected rows without truncation |
| `--patch <ROW:COL=VAL>` | Surgically patch a specific cell |
| `--drop-row <ROW_NUM>` | Surgically drop a specific row number |
| `--in-place` | Apply modifications in-place to the input file |
| `--fix, --repair, --clean` | Trigger bulk remediation engine |
| `-O, --output <PATH>` | Specify destination path for cleaned CSV |
| `--drop-duplicates` | Drop duplicate rows (keeps first occurrence) |
| `--drop-invalid` | Drop rows with unresolvable type errors |
| `--drop-outliers` | Drop rows containing IQR outliers |
| `--drop-missing` | Drop rows containing null/empty cells |
| `--fill-missing` | Impute missing numeric cells with medians |
| `--coerce-types` | Coerce recoverable types (e.g. floats to whole ints) |

---

## 🤖 Using `handler` as an AI Agent Tool

`handler` is designed to be called directly by LLM agents (Claude, GPT, Gemini, Antigravity, LangChain):

```
[Agent] ───(handler data.csv --json)───► [handler audits in <5ms]
   ▲                                              │
   │                                              ▼
   └───(Structured JSON issue report)─────────────┘
   │
   ├───► [Agent reasons on exact row & column problems]
   │
   └───(handler data.csv --patch "3:age=22" --in-place)──► [Surgically Repaired]
```

---

## 🛠️ Building from Source

```bash
git clone https://github.com/khokharsnehil45/handler.git
cd handler
cargo build --release
./target/release/handler --help
```

---

## 📄 License

Distributed under the [MIT License](LICENSE).

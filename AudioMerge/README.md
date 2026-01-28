# AudioMerge

AudioMerge is a robust Command Line Interface (CLI) tool written in Rust designed to recursively merge audio files (specifically MP3s) from subdirectories into consolidated files. It is built for efficiency, automation, and ease of organization for audiobooks, podcasts, and fragmented audio collections.

## Features

-   **Recursive Scanning**: Automatically traverses a source directory and its subdirectories to find audio files.
-   **Intelligent Grouping**: Groups files by their parent folder, creating one merged output file per folder.
-   **Natural Sorting**: Sorts files naturally (e.g., `1.mp3`, `2.mp3`, ..., `10.mp3`) to ensure correct playback order.
-   **Dry Run Mode**: Visualize exactly what files will be merged and what the output filenames will be without writing any data.
-   **Configurable Error Handling**: Choose how to handle errors during processing (`halt`, `skip`, or `prompt`).

## Installation

### Prerequisites

-   **Rust & Cargo**: You need to have the Rust programming language and Cargo build system installed. You can install them via [rustup.rs](https://rustup.rs/).

### Building from Source

1.  Clone the repository:
    ```bash
    git clone <repository-url>
    cd AudioMerge
    ```

2.  Build the project in release mode:
    ```bash
    cargo build --release
    ```

3.  The binary will be available at `./target/release/AudioMerge`.

### Installing Locally

You can install the binary directly into your Cargo bin path:

```bash
cargo install --path .
```

## Usage

Running the tool is straightforward. The general syntax is:

```bash
AudioMerge [OPTIONS]
```

### Options

-   `-s, --source <SOURCE>`: Source directory to scan for audio files. Defaults to current directory (`.`).
-   `-o, --output <OUTPUT>`: Output directory where merged files will be saved. Defaults to `output`.
-   `--dry-run`: Perform a trial run without writing any files. Prints a report of what would happen.
-   `--on-error <ON_ERROR>`: Strategy for handling errors.
    -   `skip` (Default): Log the error and continue.
    -   `halt`: Stop execution immediately upon encountering an error.
    -   `prompt`: Ask the user for input (not fully implemented in MVP).
-   `-h, --help`: Print help information.
-   `-V, --version`: Print version information.

### Examples

**1. Dry run on the current directory:**
See what files will be merged without actually doing it.

```bash
AudioMerge --dry-run
```

**2. Scan a specific library and output to a specific folder:**

```bash
AudioMerge --source ./my_audiobooks --output ./merged_books
```

**3. Strict mode (halt on error):**

```bash
AudioMerge --source ./important_data --on-error halt
```

## Development

### Running Tests

To run the test suite:

```bash
cargo test
```

### Project Structure

-   `src/main.rs`: Entry point and orchestration.
-   `src/args.rs`: CLI argument definition and parsing.
-   `src/scanner.rs`: Recursive directory scanning and file grouping logic.
-   `src/dry_run.rs`: Logic for formatting and displaying the dry-run report.

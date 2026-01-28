# Product Guidelines

## CLI Interaction & Personality
- **Communication Style**: **Informative & Descriptive**. The tool should provide detailed logs, explicitly stating which folders are being scanned, which files are identified for merging, and the progress of the operation. This ensures transparency for the user during complex recursive tasks.
- **Error Handling Strategy**: The tool must support a configurable error handling argument (e.g., `--on-error`). This argument allows the user to decide the behavior when an error or unexpected file type is encountered:
    - `halt`: Immediately stop the process and report the error.
    - `prompt`: Pause and ask the user for a decision.
    - `skip`: (Default) Log the error and continue with the next available folder or file.

## Output & Visualization
- **Dry-Run Presentation**: When running in dry-run mode, the tool should present a **Table/List Format**. This clearly maps the projected [Output Filename] to its constituent [Ordered Input Files]. This format is optimized for user verification of the merge sequence and naming logic before any writes occur.

# Initial Concept

A tool that will merge multiple audio files into one audio file and can recersivily go through subfolders combining all the auidio files in each folder into it's own single audio file. In most cases the audio file will be a mp3 and for the MVP we can assume that is the case.
The tool should:
- be written in RustLang
- be a CLI tool
- allow for the name to be drived from the folder and file names
- be able to output the combined files to a single output dir
- allow for a dry run that shows what the output file name would be and the order the files are combined in
- try and determine the correct order to combine the files in by the name of the files

# Product Definition

## Goals
- Merge multiple MP3 files into a single file recursively per folder to simplify organization.
- Provide a robust CLI interface for easy automation, scripting, and performance.
- Intelligently order files based on filenames to ensure the correct playback sequence for multi-part audio.

## Core Features (MVP)
- **Recursive Folder Processing**: Automatically traverses subdirectories, creating one consolidated audio file for each folder containing audio assets.
- **Intelligent File Sorting**: Implements smart sorting logic (e.g., natural sorting to handle numeric prefixes like "1", "02", "10") to ensure files are merged in the intended order.

## Target Audience
- **Audiobook and Podcast Listeners**: Users who frequently deal with fragmented audio chapters and need a reliable way to merge them into single, continuous files for better portability and playback experience.
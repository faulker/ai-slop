use std::fs::{self, File};
use std::io;
use std::path::Path;

pub fn merge_files(files: &[std::path::PathBuf], output_path: &Path) -> io::Result<()> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut output_file = File::create(output_path)?;

    for path in files {
        let mut input_file = File::open(path)?;
        io::copy(&mut input_file, &mut output_file)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use tempfile::TempDir;

    #[test]
    fn test_merge_files() {
        let temp_dir = TempDir::new().unwrap();
        let file1_path = temp_dir.path().join("file1.txt");
        let file2_path = temp_dir.path().join("file2.txt");
        let output_path = temp_dir.path().join("output.txt");

        let mut file1 = File::create(&file1_path).unwrap();
        file1.write_all(b"Hello, ").unwrap();

        let mut file2 = File::create(&file2_path).unwrap();
        file2.write_all(b"World!").unwrap();

        let files = vec![file1_path, file2_path];
        merge_files(&files, &output_path).unwrap();

        let mut output_file = File::open(&output_path).unwrap();
        let mut content = String::new();
        output_file.read_to_string(&mut content).unwrap();

        assert_eq!(content, "Hello, World!");
    }
}

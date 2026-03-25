use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::model::{SearchError, SupportedLanguage};

pub(crate) fn collect_supported_files(directory_path: &Path) -> Result<Vec<PathBuf>, SearchError> {
    let mut file_paths = Vec::new();
    let walker = WalkBuilder::new(directory_path)
        .standard_filters(true)
        .build();

    for entry in walker {
        let directory_entry = match entry {
            Ok(directory_entry) => directory_entry,
            Err(_) => continue,
        };

        let Some(file_type) = directory_entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        if SupportedLanguage::from_path(directory_entry.path()).is_some() {
            file_paths.push(directory_entry.into_path());
        }
    }

    file_paths.sort();
    Ok(file_paths)
}

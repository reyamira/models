use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model_refs::{BaseModelRefsFile, SCHEMA_VERSION, UPSTREAM_REPOSITORY};

#[derive(Deserialize)]
struct ProviderModelToml {
    base_model: Option<String>,
}

pub fn run(input: &Path, upstream_commit: &str, output: &Path) -> Result<(), String> {
    let providers = input.join("providers");
    if !providers.is_dir() {
        return Err(format!(
            "models.dev providers directory not found: {}",
            providers.display()
        ));
    }

    let mut refs = BTreeMap::new();
    let provider_dirs = sorted_directories(&providers)?;
    for provider_dir in provider_dirs {
        let provider_id = provider_dir
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("non-UTF-8 provider path: {}", provider_dir.display()))?;
        let models_dir = provider_dir.join("models");
        if !models_dir.is_dir() {
            continue;
        }

        let mut model_files = Vec::new();
        collect_toml_files(&models_dir, &mut model_files)?;
        model_files.sort();
        for model_file in model_files {
            let text = fs::read_to_string(&model_file)
                .map_err(|err| format!("read {}: {err}", model_file.display()))?;
            let model: ProviderModelToml = toml::from_str(&text)
                .map_err(|err| format!("parse {}: {err}", model_file.display()))?;
            let Some(base_model) = model.base_model.filter(|value| !value.is_empty()) else {
                continue;
            };
            let relative = model_file
                .strip_prefix(&models_dir)
                .map_err(|err| format!("relativize {}: {err}", model_file.display()))?;
            let mut model_id = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            model_id.truncate(model_id.len() - ".toml".len());
            let key = format!("{provider_id}/{model_id}");
            if refs.insert(key.clone(), base_model).is_some() {
                return Err(format!("duplicate provider model reference: {key}"));
            }
        }
    }

    let artifact = BaseModelRefsFile {
        schema_version: SCHEMA_VERSION,
        upstream_repository: UPSTREAM_REPOSITORY.to_string(),
        upstream_commit: upstream_commit.to_string(),
        refs,
    };
    artifact.validate()?;
    let encoded = serde_json::to_string_pretty(&artifact)
        .map_err(|err| format!("serialize base-model refs: {err}"))?
        + "\n";

    if fs::read_to_string(output).ok().as_deref() == Some(encoded.as_str()) {
        return Ok(());
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    fs::write(output, encoded).map_err(|err| format!("write {}: {err}", output.display()))
}

fn sorted_directories(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = fs::read_dir(directory)
        .map_err(|err| format!("read {}: {err}", directory.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_type().ok()?.is_dir().then_some(entry.path()))
        .collect::<Vec<_>>();
    result.sort();
    Ok(result)
}

fn collect_toml_files(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in
        fs::read_dir(directory).map_err(|err| format!("read {}: {err}", directory.display()))?
    {
        let entry = entry.map_err(|err| format!("read {} entry: {err}", directory.display()))?;
        let path = entry.path();
        // `metadata` follows symlinks, matching models.dev's Bun.Glob
        // `followSymlinks: true`. Broken symlinks are absent from Bun's scan
        // and are skipped here as well.
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            collect_toml_files(&path, output)?;
        } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == "toml") {
            output.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_path_keyed_base_model_refs() {
        let root = std::env::temp_dir().join(format!(
            "modelsdev-model-refs-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let models = root.join("providers/bedrock/models/anthropic");
        fs::create_dir_all(&models).expect("create fixture directories");
        fs::write(
            models.join("claude-opus.toml"),
            "base_model = \"anthropic/claude-opus\"\nname = \"Claude Opus\"\n",
        )
        .expect("write linked fixture");
        fs::write(models.join("standalone.toml"), "name = \"Standalone\"\n")
            .expect("write standalone fixture");
        let output = root.join("refs.json");
        let commit = "0123456789abcdef0123456789abcdef01234567";

        run(&root, commit, &output).expect("generate refs");
        let artifact: BaseModelRefsFile =
            serde_json::from_str(&fs::read_to_string(&output).expect("read generated refs"))
                .expect("parse generated refs");
        assert_eq!(artifact.upstream_commit, commit);
        assert_eq!(artifact.refs.len(), 1);
        assert_eq!(
            artifact.refs.get("bedrock/anthropic/claude-opus"),
            Some(&"anthropic/claude-opus".to_string())
        );

        fs::remove_dir_all(root).expect("remove fixture directory");
    }

    #[cfg(unix)]
    #[test]
    fn follows_provider_model_symlinks_like_upstream_build() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "modelsdev-model-refs-symlink-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let models = root.join("providers/alias-provider/models");
        let shared = root.join("shared");
        fs::create_dir_all(&models).expect("create provider fixture");
        fs::create_dir_all(&shared).expect("create shared fixture");
        let target = shared.join("canonical.toml");
        fs::write(&target, "base_model = \"lab/canonical\"\n").expect("write symlink target");
        symlink(&target, models.join("alias.toml")).expect("create model symlink");
        let output = root.join("refs.json");

        run(&root, "0123456789abcdef0123456789abcdef01234567", &output).expect("generate refs");
        let artifact: BaseModelRefsFile =
            serde_json::from_str(&fs::read_to_string(&output).expect("read generated refs"))
                .expect("parse generated refs");
        assert_eq!(
            artifact.refs.get("alias-provider/alias"),
            Some(&"lab/canonical".to_string())
        );

        fs::remove_dir_all(root).expect("remove fixture directory");
    }
}

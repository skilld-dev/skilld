use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct PnpmPackage {
    pub name: String,
    pub version: String,
}

/// A pnpm prefix alone never establishes ownership. Check the link and package metadata.
pub(crate) fn linked_package(link: &Path, canonical: &Path) -> Option<PnpmPackage> {
    if !fs::symlink_metadata(link).ok()?.file_type().is_symlink() {
        return None;
    }
    let skills = canonical.parent()?;
    if skills.file_name()? != "skills"
        || !canonical
            .components()
            .any(|part| part.as_os_str() == "node_modules")
    {
        return None;
    }
    let manifest = skills.parent()?.join("package.json");
    if fs::metadata(&manifest).ok()?.len() > 64 * 1024 {
        return None;
    }
    let package: PnpmPackage = serde_json::from_slice(&fs::read(manifest).ok()?).ok()?;
    if package.name.is_empty() || package.version.is_empty() {
        return None;
    }
    let skill = canonical.file_name()?.to_str()?;
    let expected = format!("pnpm-{}-{skill}", package.name.replace('/', "+"));
    (link.file_name()?.to_str()? == expected).then_some(package)
}

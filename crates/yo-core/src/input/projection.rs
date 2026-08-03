use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{DefaultIgnorableCodePoint, GeneralCategory},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{SkillReference, WorkspaceReference, WorkspaceReferenceKind};

pub fn workspace_reference_projection(reference: &WorkspaceReference) -> String {
    let suffix = match reference.kind() {
        WorkspaceReferenceKind::File => "",
        WorkspaceReferenceKind::Directory => "/",
    };
    format!("@{}{suffix}", visible_selector(reference.relative_path()))
}

pub fn skill_reference_projection(reference: &SkillReference) -> String {
    format!("${}", visible_selector(reference.name()))
}

fn visible_selector(raw: &str) -> String {
    raw.graphemes(true)
        .map(|cluster| {
            if cluster.contains('\\') || !is_visible_cluster(cluster) {
                cluster.chars().map(escape).collect()
            } else {
                cluster.to_owned()
            }
        })
        .collect()
}

fn is_visible_cluster(cluster: &str) -> bool {
    !cluster.chars().any(char::is_control)
        && cluster.chars().any(|character| {
            let category = CodePointMapData::<GeneralCategory>::new().get(character);
            !matches!(
                category,
                GeneralCategory::NonspacingMark
                    | GeneralCategory::SpacingMark
                    | GeneralCategory::EnclosingMark
            ) && !CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(character)
        })
}

fn escape(character: char) -> String {
    format!("\\u{{{:X}}}", u32::from(character))
}

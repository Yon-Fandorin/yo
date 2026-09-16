use super::tool_files::literal_block;

pub(super) fn image_markdown(mime: &str, data: &str) -> String {
    if matches!(mime, "image/png" | "image/jpeg")
        && !data.is_empty()
        && data
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+/=".contains(&byte))
    {
        format!("![Image output](data:{mime};base64,{data})")
    } else {
        literal_block(
            "text",
            "Image output · unsupported or invalid media (original retained)",
        )
    }
}

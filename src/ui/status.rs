use crate::app::update::Status;

pub fn label(status: &Status) -> String {
    match status {
        Status::Ready => "Ready".to_owned(),
        Status::Working => "Working".to_owned(),
        Status::Tool(description) => format!("Tool: {description}"),
    }
}

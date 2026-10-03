use crate::app::update::Status;

pub fn label(status: &Status) -> String {
    match status {
        Status::Ready => "Ready".to_owned(),
        Status::Working => "Working".to_owned(),
        Status::Tool(description) => format!("Tool: {description}"),
        Status::Connecting => "Connecting".to_owned(),
        Status::Streaming => "Streaming".to_owned(),
        Status::Completed => "Completed".to_owned(),
        Status::Failed => "Failed".to_owned(),
        Status::Cancelled => "Cancelled".to_owned(),
    }
}

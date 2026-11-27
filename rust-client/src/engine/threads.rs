use super::{Failure, Message, media::Sources, messaging};
use matrix_sdk::{
    Client, Room,
    room::{IncludeRelations, ListThreadsOptions, RelationsOptions},
    ruma::{
        UInt,
        api::client::threads::get_threads::v1::IncludeThreads,
        events::{
            relation::{RelationType, Thread},
            room::message::{Relation, RoomMessageEventContent},
        },
    },
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Request {
    Pins,
    SetPin {
        event: String,
        pinned: bool,
    },
    Threads {
        before: Option<String>,
    },
    Replies {
        root: String,
        before: Option<String>,
    },
    ReplyImage {
        root: String,
        latest: String,
        content: serde_json::Value,
    },
    Reply {
        root: String,
        latest: String,
        body: String,
    },
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Page {
    pub messages: Vec<Message>,
    pub before: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Response {
    Pins(Vec<super::pins::Pin>),
    PinChanged,
    Threads(Page),
    Replies {
        root: Option<Box<Message>>,
        page: Page,
    },
    Sent(String),
}

const PAGE_SIZE: u32 = 30;

pub(super) async fn request(
    client: &Client,
    id: &str,
    action: Request,
    sources: &Sources,
) -> Result<Response, Failure> {
    let room = messaging::room(client, id)?;
    let me = client.user_id().map(|id| id.as_str()).unwrap_or_default();
    match action {
        Request::Pins => super::pins::list(&room, me, sources)
            .await
            .map(Response::Pins),
        Request::SetPin { event, pinned } => {
            super::pins::set(&room, &event, pinned).await?;
            Ok(Response::PinChanged)
        }
        Request::Threads { before } => {
            let roots = room
                .list_threads(ListThreadsOptions {
                    include_threads: IncludeThreads::All,
                    from: before,
                    limit: Some(UInt::from(PAGE_SIZE)),
                })
                .await
                .map_err(|err| messaging::http(&err))?;
            Ok(Response::Threads(Page {
                messages: roots
                    .chunk
                    .iter()
                    .zip(1..)
                    .filter_map(|(event, row)| messaging::message(event, row, me, sources))
                    .collect(),
                before: roots.prev_batch_token,
            }))
        }
        Request::Replies { root, before } => replies(&room, &root, before, me, sources).await,
        Request::Reply { root, latest, body } => {
            let content = reply_content(&root, &latest, &body)?;
            send_reply(&room, content).await
        }
        Request::ReplyImage {
            root,
            latest,
            content,
        } => {
            if room.encryption_state().is_encrypted() && !content["file"].is_object() {
                return Err(Failure::Failed);
            }
            let mut content: RoomMessageEventContent =
                serde_json::from_value(content).map_err(|_| Failure::Failed)?;
            if !matches!(
                content.msgtype,
                matrix_sdk::ruma::events::room::message::MessageType::Image(_)
            ) {
                return Err(Failure::Failed);
            }
            relate(&mut content, &root, &latest)?;
            send_reply(&room, content).await
        }
    }
}

async fn replies(
    room: &Room,
    root: &str,
    before: Option<String>,
    me: &str,
    sources: &Sources,
) -> Result<Response, Failure> {
    let id = messaging::event_id(root)?;
    let message = if before.is_none() {
        let event = room
            .load_or_fetch_event(&id, None)
            .await
            .map_err(|err| messaging::http(&err))?;
        messaging::message(&event, 1, me, sources).map(Box::new)
    } else {
        None
    };
    let relations = room
        .relations(
            id,
            RelationsOptions {
                from: before,
                limit: Some(UInt::from(PAGE_SIZE)),
                include_relations: IncludeRelations::RelationsOfType(RelationType::Thread),
                ..Default::default()
            },
        )
        .await
        .map_err(|err| messaging::http(&err))?;
    Ok(Response::Replies {
        root: message,
        page: Page {
            messages: relations
                .chunk
                .iter()
                .rev()
                .zip(2..)
                .filter_map(|(event, row)| messaging::message(event, row, me, sources))
                .collect(),
            before: relations.next_batch_token,
        },
    })
}

fn reply_content(root: &str, latest: &str, body: &str) -> Result<RoomMessageEventContent, Failure> {
    if body.trim().is_empty() {
        return Err(Failure::Failed);
    }
    let mut content = RoomMessageEventContent::text_markdown(body);
    relate(&mut content, root, latest)?;
    Ok(content)
}

fn relate(content: &mut RoomMessageEventContent, root: &str, latest: &str) -> Result<(), Failure> {
    content.relates_to = Some(Relation::Thread(Thread::plain(
        messaging::event_id(root)?,
        messaging::event_id(latest)?,
    )));
    Ok(())
}

async fn send_reply(room: &Room, content: RoomMessageEventContent) -> Result<Response, Failure> {
    let sent = room
        .send(content)
        .await
        .map_err(|err| messaging::http(&err))?;
    Ok(Response::Sent(sent.response.event_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_reply_addresses_root_and_keeps_latest_as_fallback() {
        let event =
            serde_json::to_value(reply_content("$root", "$latest", "Hello").unwrap()).unwrap();
        assert_eq!(event["m.relates_to"]["rel_type"], "m.thread");
        assert_eq!(event["m.relates_to"]["event_id"], "$root");
        assert_eq!(
            event["m.relates_to"]["m.in_reply_to"]["event_id"],
            "$latest"
        );
        assert_eq!(event["m.relates_to"]["is_falling_back"], true);
        assert!(reply_content("$root", "$latest", " ").is_err());
        assert!(reply_content("invalid", "$latest", "Hello").is_err());
    }
}

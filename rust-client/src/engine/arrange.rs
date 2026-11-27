//! Arranging a space's channels and categories (ROADMAP N5): a child's
//! `m.space.child` edge gets a new `order`, or moves to another space. The
//! App works out the orders (channel_order.rs); this writes them.
use super::{
    Failure,
    messaging::{http, room},
};
use matrix_sdk::{
    Client, Room, deserialized_responses::RawAnySyncOrStrippedState, ruma::events::StateEventType,
};
use serde_json::{Value, json};

/// One edge to rewrite.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Change {
    /// Give `child`'s edge in `space` this `order`, keeping its other fields.
    Order {
        space: String,
        child: String,
        order: String,
    },
    /// Add `child` to `to` at `order`, with the `via` and `suggested` of its
    /// edge in `from`, then take it out of `from`: a failure in between
    /// leaves it in both rather than in neither.
    Move {
        from: String,
        to: String,
        child: String,
        order: String,
    },
}

pub async fn arrange(client: &Client, changes: Vec<Change>) -> Result<(), Failure> {
    for change in changes {
        match change {
            Change::Order {
                space,
                child,
                order,
            } => {
                let space = room(client, &space)?;
                let content = with_order(edge(&space, &child).await?, order);
                send(&space, &child, content).await?;
            }
            Change::Move {
                from,
                to,
                child,
                order,
            } => {
                let (old, new) = (room(client, &from)?, room(client, &to)?);
                let content = with_order(edge(&old, &child).await?, order);
                send(&new, &child, content).await?;
                // An edge without `via` is no edge.
                send(&old, &child, json!({})).await?;
            }
        }
    }
    Ok(())
}

/// The child's current edge in `space`, from the store, with the fields the
/// spec doesn't name kept.
async fn edge(space: &Room, child: &str) -> Result<Value, Failure> {
    let state = space
        .get_state_event(StateEventType::SpaceChild, child)
        .await
        .map_err(|_| Failure::Unavailable)?;
    let Some(RawAnySyncOrStrippedState::Sync(raw)) = state else {
        return Err(Failure::Unavailable);
    };
    let event: Value = raw.deserialize_as().map_err(|_| Failure::Unavailable)?;
    let content = event["content"].clone();
    if content["via"].as_array().is_none_or(Vec::is_empty) {
        return Err(Failure::Unavailable);
    }
    Ok(content)
}

fn with_order(mut content: Value, order: String) -> Value {
    content["order"] = order.into();
    content
}

async fn send(space: &Room, child: &str, content: Value) -> Result<(), Failure> {
    space
        .send_state_event_raw("m.space.child", child, content)
        .await
        .map(|_| ())
        .map_err(|e| http(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_order_keeps_the_edge_s_other_fields() {
        let edge = json!({"via": ["h"], "suggested": true, "org.example": 1, "order": "a"});
        assert_eq!(
            with_order(edge, "b".into()),
            json!({"via": ["h"], "suggested": true, "org.example": 1, "order": "b"})
        );
    }
}

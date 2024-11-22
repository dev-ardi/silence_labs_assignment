use core::panic;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use dashmap::{DashMap, Entry};
use tokio::sync::oneshot;
use tokio::time::timeout;

/// This displays a cute message
fn craft_response(t0: Instant, t1: Instant) -> impl IntoResponse {
    (
        StatusCode::OK,
        format!(
            "The time elapsed between the API calls was {:?}. Thanks for using our services! <3 ",
            t1.duration_since(t0)
        ),
    )
}

/// This endpoint allows two parties to sync. When one party makes a POST request, the response will be delayed until the second party requests the same URL. In other words, the first party is blocked until the second party arrives or a timeout occurs (let it be 10 seconds).
#[axum::debug_handler]
async fn wait_for_second_party(
    State(state): State<AppState>,
    Path(unique_id): Path<UniqueID>,
) -> impl IntoResponse {
    // First let's see if we're the first or the second party.
    match state.requests.entry(unique_id) {
        Entry::Vacant(vacant_entry) => {
            // We're the first request, let's create a new id and put it in the dashmap.
            let (tx, rx) = oneshot::channel();
            let t0 = Instant::now();
            vacant_entry.insert((tx, t0));
            // Let's wait for a second request for 10 seconds;
            let res = timeout(Duration::from_secs(10), async move {
                rx.await
                    // This is impossible:
                    // 1. We drop the sender if we didn't get anything.
                    // 2. The second request drops the entry after sending it.
                    .expect("The sender cannot be dropped")
            })
            .await;
            match res {
                Ok(t1) => craft_response(t0, t1).into_response(),
                Err(_) => {
                    // At this point we have to drop the entry.
                    _ = state.requests.remove(&unique_id);

                    (
                        StatusCode::REQUEST_TIMEOUT,
                        "Did not receive a second request :'(",
                    )
                        .into_response()
                }
            }
        }
        Entry::Occupied(occupied_entry) => {
            // We're the second request! Let's return then.
            let (_, (canceller, t0)) = occupied_entry.remove_entry();
            let t1 = Instant::now();
            canceller
                .send(t1)
                // This is impossible:
                // The race condition is impossible since the entry is locked
                // and the cancel timer cannot delete it.
                .expect("Channel was closed?!");
            craft_response(t0, t1).into_response()
        }
    }
}

type Canceller = oneshot::Sender<Instant>;
type UniqueID = u64;

#[derive(Clone, Default)]
struct AppState {
    // Let's use dashmap since it's the easier solution.
    // We could use other systems but this is good enough for an initial implementation.
    requests: Arc<DashMap<UniqueID, (Canceller, Instant)>>,
}

#[tokio::main]
async fn main() {
    let router = Router::new()
        // `POST /users` goes to `create_user`
        .route(
            "/wait-for-second-party/:unique-id",
            post(wait_for_second_party),
        )
        .with_state(AppState::default());

    // run our app with hyper, listening globally on port 3000
    let address = "0.0.0.0:3000";
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    println!("Running sync server on {address}");
    axum::serve(listener, router).await.unwrap();
}

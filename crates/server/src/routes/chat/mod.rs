pub mod agents;
pub mod messages;
pub mod runs;
pub mod sessions;

use axum::{
    Router,
    middleware::from_fn_with_state,
    routing::get,
};

use crate::{DeploymentImpl, middleware::{load_chat_agent_middleware, load_chat_session_middleware}};

pub fn router(deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    let session_router = Router::new()
        .route(
            "/",
            get(sessions::get_session)
                .put(sessions::update_session)
                .delete(sessions::delete_session),
        )
        .route("/stream", get(sessions::stream_session_ws))
        .route(
            "/agents",
            get(sessions::get_session_agents).post(sessions::create_session_agent),
        )
        .route(
            "/agents/{session_agent_id}",
            axum::routing::put(sessions::update_session_agent)
                .delete(sessions::delete_session_agent),
        )
        .route(
            "/messages",
            get(messages::get_messages).post(messages::create_message),
        )
        .layer(from_fn_with_state(
            deployment.clone(),
            load_chat_session_middleware,
        ));

    let sessions_router = Router::new()
        .route("/", get(sessions::get_sessions).post(sessions::create_session))
        .nest("/{session_id}", session_router);

    let agent_router = Router::new()
        .route(
            "/",
            get(agents::get_agent)
                .put(agents::update_agent)
                .delete(agents::delete_agent),
        )
        .layer(from_fn_with_state(
            deployment.clone(),
            load_chat_agent_middleware,
        ));

    let agents_router = Router::new()
        .route("/", get(agents::get_agents).post(agents::create_agent))
        .nest("/{agent_id}", agent_router);

    let messages_router = Router::new()
        .route("/{message_id}", get(messages::get_message).delete(messages::delete_message));

    Router::new().nest(
        "/chat",
        Router::new()
            .nest("/sessions", sessions_router)
            .nest("/agents", agents_router)
            .nest("/messages", messages_router)
            .route("/runs/{run_id}/log", get(runs::get_run_log))
            .route("/runs/{run_id}/diff", get(runs::get_run_diff))
            .route(
                "/runs/{run_id}/untracked",
                get(runs::get_run_untracked_file),
            ),
    )
}

use std::{
    collections::HashSet,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use rmcp::{
    RoleClient,
    model::{
        CallToolRequest, CallToolRequestParams, CallToolResult, ClientJsonRpcMessage,
        ClientNotification, ClientRequest, JsonObject, JsonRpcMessage, ListToolsRequest,
        ListToolsResult, PaginatedRequestParams, PingRequest, ProgressNotificationParam, RequestId,
        ServerJsonRpcMessage, ServerNotification, ServerResult,
    },
    service::{Peer, PeerRequestOptions, RequestHandle, RunningService, ServiceError},
    transport::Transport,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{
    McpError, McpErrorCode, framing,
    progress::{ProgressInbox, ProgressRouter},
    server::McpHandshakeSnapshot,
};

#[derive(Debug, Clone)]
pub(crate) struct TransportShared {
    state: Arc<Mutex<TransportState>>,
}

impl TransportShared {
    pub(crate) fn new(max_in_flight: usize, max_notifications: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(TransportState {
                outbound_requests: HashSet::with_capacity(max_in_flight),
                inbound_requests: HashSet::new(),
                notification_streak: 0,
                max_in_flight,
                max_notifications,
                failure: None,
                catalog_stale: false,
            })),
        }
    }

    pub(crate) fn failure_code(&self) -> Option<McpErrorCode> {
        self.lock().failure
    }

    pub(crate) fn catalog_stale(&self) -> bool {
        self.lock().catalog_stale
    }

    fn fail(&self, code: McpErrorCode) -> TransportError {
        let mut state = self.lock();
        state.failure.get_or_insert(code);
        TransportError::new(code)
    }

    fn lock(&self) -> MutexGuard<'_, TransportState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug)]
struct TransportState {
    outbound_requests: HashSet<RequestId>,
    inbound_requests: HashSet<RequestId>,
    notification_streak: usize,
    max_in_flight: usize,
    max_notifications: usize,
    failure: Option<McpErrorCode>,
    catalog_stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransportError {
    code: McpErrorCode,
}

impl TransportError {
    const fn new(code: McpErrorCode) -> Self {
        Self { code }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&McpError::new(self.code), formatter)
    }
}

impl std::error::Error for TransportError {}

pub(crate) struct BoundedStdioTransport<R, W> {
    reader: framing::BoundedFrameReader<R>,
    writer: Arc<AsyncMutex<Option<W>>>,
    shared: TransportShared,
    max_frame_bytes: usize,
    frame_timeout: Duration,
    progress: ProgressRouter,
}

impl<R, W> BoundedStdioTransport<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    pub(crate) fn new(
        reader: R,
        writer: W,
        shared: TransportShared,
        max_frame_bytes: usize,
        frame_timeout: Duration,
        progress: ProgressRouter,
    ) -> Self {
        Self {
            reader: framing::BoundedFrameReader::new(reader, max_frame_bytes, frame_timeout),
            writer: Arc::new(AsyncMutex::new(Some(writer))),
            shared,
            max_frame_bytes,
            frame_timeout,
            progress,
        }
    }
}

impl<R, W> Transport<RoleClient> for BoundedStdioTransport<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Error = TransportError;

    fn send(
        &mut self,
        item: ClientJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let writer = self.writer.clone();
        let shared = self.shared.clone();
        let max_frame_bytes = self.max_frame_bytes;
        let frame_timeout = self.frame_timeout;
        async move {
            let frame = framing::serialize(&item, max_frame_bytes)
                .map_err(|error| shared.fail(frame_error_code(error)))?;
            let outbound_id = validate_outbound(&shared, &item)?;
            let write_result = tokio::time::timeout(frame_timeout, async {
                let mut writer = writer.lock().await;
                let writer = writer
                    .as_mut()
                    .ok_or_else(|| TransportError::new(McpErrorCode::Transport))?;
                writer
                    .write_all(&frame)
                    .await
                    .map_err(|_| TransportError::new(McpErrorCode::Transport))?;
                writer
                    .flush()
                    .await
                    .map_err(|_| TransportError::new(McpErrorCode::Transport))
            })
            .await
            .map_err(|_| TransportError::new(McpErrorCode::Timeout))
            .and_then(|result| result);
            if let Err(error) = write_result {
                if let Some(id) = outbound_id {
                    shared.lock().outbound_requests.remove(&id);
                }
                return Err(shared.fail(error.code));
            }
            Ok::<(), TransportError>(())
        }
    }

    async fn receive(&mut self) -> Option<ServerJsonRpcMessage> {
        let message = match self.reader.read().await {
            Ok(Some(message)) => message,
            Ok(None) => {
                self.shared.fail(McpErrorCode::ServerExit);
                return None;
            }
            Err(error) => {
                self.shared.fail(frame_error_code(error));
                return None;
            }
        };
        if validate_inbound(&self.shared, &message).is_err() {
            return None;
        }
        route_progress(&self.progress, &message);
        Some(message)
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let writer = self.writer.clone();
        tokio::time::timeout(self.frame_timeout, async move {
            let mut writer = writer.lock().await;
            if let Some(mut writer) = writer.take() {
                writer
                    .shutdown()
                    .await
                    .map_err(|_| TransportError::new(McpErrorCode::Transport))?;
            }
            Ok::<(), TransportError>(())
        })
        .await
        .map_err(|_| self.shared.fail(McpErrorCode::Timeout))?
        .map_err(|error| self.shared.fail(error.code))
    }
}

pub(crate) trait McpClientTransport:
    Transport<RoleClient, Error = TransportError> + Send + 'static
{
}

impl<T> McpClientTransport for T where
    T: Transport<RoleClient, Error = TransportError> + Send + 'static
{
}

pub(crate) struct SdkClient {
    service: RunningService<RoleClient, ()>,
    execution: SdkExecutionHandle,
    cancellation_task: Option<JoinHandle<()>>,
}

impl SdkClient {
    pub(crate) async fn connect<T: McpClientTransport>(
        transport: T,
        transport_shared: TransportShared,
        cancellation: CancellationToken,
        max_in_flight: usize,
        max_progress_events: usize,
        cancellation_timeout: Duration,
        progress: ProgressRouter,
    ) -> Result<Self, McpError> {
        let service_cancellation = cancellation.clone();
        rmcp::service::serve_client_with_ct((), transport, cancellation)
            .await
            .map(|service| {
                let (cancellation_tx, cancellation_rx) = mpsc::channel(max_in_flight);
                let cancellation_task = tokio::spawn(dispatch_cancellations(
                    cancellation_rx,
                    service_cancellation.clone(),
                ));
                let execution = SdkExecutionHandle {
                    peer: service.peer().clone(),
                    progress,
                    permits: Arc::new(Semaphore::new(max_in_flight)),
                    max_progress_events,
                    service_cancellation,
                    cancellation_tx,
                    cancellation_timeout,
                    transport: transport_shared,
                };
                Self {
                    service,
                    execution,
                    cancellation_task: Some(cancellation_task),
                }
            })
            .map_err(|_| McpError::new(McpErrorCode::Handshake))
    }

    pub(crate) fn execution_handle(&self) -> SdkExecutionHandle {
        self.execution.clone()
    }

    pub(crate) fn handshake_snapshot(&self) -> Result<McpHandshakeSnapshot, McpError> {
        let info = self
            .service
            .peer()
            .peer_info()
            .ok_or_else(|| McpError::new(McpErrorCode::Handshake))?;
        let implementation = serde_json::to_value(&info.server_info)
            .map_err(|_| McpError::new(McpErrorCode::Descriptor))?;
        McpHandshakeSnapshot::freeze(info.protocol_version.as_str(), implementation)
    }

    pub(crate) async fn probe(&self, timeout: Duration) -> Result<(), McpError> {
        let request = ClientRequest::PingRequest(PingRequest::default());
        let handle = self
            .service
            .peer()
            .send_cancellable_request(request, PeerRequestOptions::with_timeout(timeout))
            .await
            .map_err(|error| map_service_error(&error))?;
        match handle
            .await_response()
            .await
            .map_err(|error| map_service_error(&error))?
        {
            ServerResult::EmptyResult(_) => Ok(()),
            _ => Err(McpError::new(McpErrorCode::Transport)),
        }
    }

    pub(crate) async fn list_tools_page(
        &self,
        cursor: Option<String>,
        timeout: Duration,
    ) -> Result<ListToolsResult, McpError> {
        let params = PaginatedRequestParams::default().with_cursor(cursor);
        let request = ClientRequest::ListToolsRequest(ListToolsRequest::with_param(params));
        let handle = self
            .service
            .peer()
            .send_cancellable_request(request, PeerRequestOptions::with_timeout(timeout))
            .await
            .map_err(|error| map_service_error(&error))?;
        match handle
            .await_response()
            .await
            .map_err(|error| map_service_error(&error))?
        {
            ServerResult::ListToolsResult(result) => Ok(result),
            _ => Err(McpError::new(McpErrorCode::Transport)),
        }
    }

    pub(crate) async fn close(&mut self) -> Result<(), McpError> {
        let service_result = self
            .service
            .close()
            .await
            .map(|_| ())
            .map_err(|_| McpError::new(McpErrorCode::Shutdown));
        self.execution.abort_service();
        if let Some(task) = self.cancellation_task.take() {
            task.await
                .map_err(|_| McpError::new(McpErrorCode::Shutdown))?;
        }
        service_result
    }

    pub(crate) fn cancel(&self) {
        self.service.cancellation_token().cancel();
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SdkExecutionHandle {
    peer: Peer<RoleClient>,
    progress: ProgressRouter,
    permits: Arc<Semaphore>,
    max_progress_events: usize,
    service_cancellation: CancellationToken,
    cancellation_tx: mpsc::Sender<SdkCallCancellation>,
    cancellation_timeout: Duration,
    transport: TransportShared,
}

impl SdkExecutionHandle {
    pub(crate) fn failure_code(&self) -> Option<McpErrorCode> {
        self.transport.failure_code()
    }

    pub(crate) fn catalog_stale(&self) -> bool {
        self.transport.catalog_stale()
    }

    pub(crate) async fn acquire(&self) -> Result<OwnedSemaphorePermit, McpError> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| McpError::new(McpErrorCode::Cancellation))
    }

    pub(crate) async fn begin_tool_call(
        &self,
        remote_name: String,
        arguments: JsonObject,
        permit: OwnedSemaphorePermit,
    ) -> Result<SdkToolCall, McpError> {
        let params = CallToolRequestParams::new(remote_name).with_arguments(arguments);
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let handle = self
            .peer
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .map_err(|error| map_service_error(&error))?;
        let inbox = self
            .progress
            .subscribe(handle.progress_token.clone(), self.max_progress_events)?;
        Ok(SdkToolCall {
            handle: Some(handle),
            inbox,
            progress: self.progress.clone(),
            service_cancellation: self.service_cancellation.clone(),
            cancellation_tx: self.cancellation_tx.clone(),
            cancellation_timeout: self.cancellation_timeout,
            transport: self.transport.clone(),
            terminal: false,
            permit: Some(permit),
        })
    }

    pub(crate) fn abort_service(&self) {
        self.service_cancellation.cancel();
    }
}

#[derive(Debug)]
pub(crate) struct SdkToolCall {
    handle: Option<RequestHandle<RoleClient>>,
    inbox: ProgressInbox,
    progress: ProgressRouter,
    service_cancellation: CancellationToken,
    cancellation_tx: mpsc::Sender<SdkCallCancellation>,
    cancellation_timeout: Duration,
    transport: TransportShared,
    terminal: bool,
    permit: Option<OwnedSemaphorePermit>,
}

impl SdkToolCall {
    pub(crate) async fn next(&mut self) -> SdkToolCallEvent {
        loop {
            let raw = {
                let handle = self.handle.as_mut().expect("active MCP call has a handle");
                tokio::select! {
                    biased;
                    () = self.inbox.overflow.cancelled() => RawSdkToolCallEvent::ProgressOverflow,
                    progress = self.inbox.receiver.recv(), if self.inbox.open => {
                        RawSdkToolCallEvent::Progress(progress)
                    }
                    response = &mut handle.rx => RawSdkToolCallEvent::Response(Box::new(response)),
                }
            };
            match raw {
                RawSdkToolCallEvent::Progress(Some(progress)) => {
                    return SdkToolCallEvent::Progress(progress);
                }
                RawSdkToolCallEvent::Progress(None) => self.inbox.open = false,
                RawSdkToolCallEvent::ProgressOverflow => {
                    return SdkToolCallEvent::ProgressOverflow;
                }
                RawSdkToolCallEvent::Response(response) => {
                    self.terminal = true;
                    self.progress.unsubscribe(&self.inbox.token);
                    let result = match *response {
                        Ok(Ok(ServerResult::CallToolResult(result))) => Ok(result),
                        Ok(Ok(_)) | Err(_) => Err(McpError::new(
                            self.transport
                                .failure_code()
                                .unwrap_or(McpErrorCode::Transport),
                        )),
                        Ok(Err(error)) => Err(map_service_error(&error)),
                    };
                    return SdkToolCallEvent::Result(result);
                }
            }
        }
    }

    pub(crate) async fn cancel(&mut self, timeout: Duration) -> Result<(), McpError> {
        self.terminal = true;
        self.progress.unsubscribe(&self.inbox.token);
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        match tokio::time::timeout(
            timeout,
            handle.cancel(Some("host cancelled MCP tool call".to_owned())),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.service_cancellation.cancel();
                Err(map_service_error(&error))
            }
            Err(_) => {
                self.service_cancellation.cancel();
                Err(McpError::new(McpErrorCode::Timeout))
            }
        }
    }
}

impl Drop for SdkToolCall {
    fn drop(&mut self) {
        self.progress.unsubscribe(&self.inbox.token);
        if self.terminal {
            return;
        }
        let (Some(handle), Some(permit)) = (self.handle.take(), self.permit.take()) else {
            return;
        };
        let cancellation = SdkCallCancellation {
            handle,
            timeout: self.cancellation_timeout,
            _permit: permit,
        };
        if self.cancellation_tx.try_send(cancellation).is_err() {
            self.service_cancellation.cancel();
        }
    }
}

#[derive(Debug)]
struct SdkCallCancellation {
    handle: RequestHandle<RoleClient>,
    timeout: Duration,
    _permit: OwnedSemaphorePermit,
}

async fn dispatch_cancellations(
    mut receiver: mpsc::Receiver<SdkCallCancellation>,
    service_cancellation: CancellationToken,
) {
    loop {
        let cancellation = tokio::select! {
            biased;
            () = service_cancellation.cancelled() => return,
            cancellation = receiver.recv() => cancellation,
        };
        let Some(cancellation) = cancellation else {
            return;
        };
        let result = tokio::time::timeout(
            cancellation.timeout,
            cancellation
                .handle
                .cancel(Some("host dropped MCP tool call".to_owned())),
        )
        .await;
        if !matches!(result, Ok(Ok(()))) {
            service_cancellation.cancel();
            return;
        }
    }
}

enum RawSdkToolCallEvent {
    Progress(Option<ProgressNotificationParam>),
    ProgressOverflow,
    Response(
        Box<Result<Result<ServerResult, ServiceError>, tokio::sync::oneshot::error::RecvError>>,
    ),
}

pub(crate) enum SdkToolCallEvent {
    Progress(ProgressNotificationParam),
    ProgressOverflow,
    Result(Result<CallToolResult, McpError>),
}

fn route_progress(progress: &ProgressRouter, message: &ServerJsonRpcMessage) {
    let JsonRpcMessage::Notification(notification) = message else {
        return;
    };
    let ServerNotification::ProgressNotification(notification) = &notification.notification else {
        return;
    };
    progress.route(notification.params.clone());
}

fn validate_outbound(
    shared: &TransportShared,
    message: &ClientJsonRpcMessage,
) -> Result<Option<RequestId>, TransportError> {
    let mut state = shared.lock();
    if let Some(code) = state.failure {
        return Err(TransportError::new(code));
    }
    match message {
        JsonRpcMessage::Request(request) => {
            if state.outbound_requests.len() >= state.max_in_flight
                || !state.outbound_requests.insert(request.id.clone())
            {
                state.failure = Some(McpErrorCode::Transport);
                return Err(TransportError::new(McpErrorCode::Transport));
            }
            Ok(Some(request.id.clone()))
        }
        JsonRpcMessage::Response(response) => {
            if !state.inbound_requests.remove(&response.id) {
                state.failure = Some(McpErrorCode::Transport);
                return Err(TransportError::new(McpErrorCode::Transport));
            }
            Ok(None)
        }
        JsonRpcMessage::Error(error) => {
            if let Some(id) = &error.id
                && !state.inbound_requests.remove(id)
            {
                state.failure = Some(McpErrorCode::Transport);
                return Err(TransportError::new(McpErrorCode::Transport));
            }
            Ok(None)
        }
        JsonRpcMessage::Notification(notification) => {
            if let ClientNotification::CancelledNotification(cancelled) = &notification.notification
                && let Some(id) = &cancelled.params.request_id
            {
                state.outbound_requests.remove(id);
            }
            Ok(None)
        }
    }
}

fn validate_inbound(
    shared: &TransportShared,
    message: &ServerJsonRpcMessage,
) -> Result<(), TransportError> {
    let mut state = shared.lock();
    if let Some(code) = state.failure {
        return Err(TransportError::new(code));
    }
    match message {
        JsonRpcMessage::Response(response) => {
            state.notification_streak = 0;
            if !state.outbound_requests.remove(&response.id) {
                state.failure = Some(McpErrorCode::Transport);
                return Err(TransportError::new(McpErrorCode::Transport));
            }
        }
        JsonRpcMessage::Error(error) => {
            state.notification_streak = 0;
            let Some(id) = &error.id else {
                state.failure = Some(McpErrorCode::Transport);
                return Err(TransportError::new(McpErrorCode::Transport));
            };
            if !state.outbound_requests.remove(id) {
                state.failure = Some(McpErrorCode::Transport);
                return Err(TransportError::new(McpErrorCode::Transport));
            }
        }
        JsonRpcMessage::Request(request) => {
            state.notification_streak = 0;
            if !state.inbound_requests.insert(request.id.clone()) {
                state.failure = Some(McpErrorCode::Transport);
                return Err(TransportError::new(McpErrorCode::Transport));
            }
        }
        JsonRpcMessage::Notification(notification) => {
            state.notification_streak = state.notification_streak.saturating_add(1);
            if state.notification_streak > state.max_notifications {
                state.failure = Some(McpErrorCode::Transport);
                return Err(TransportError::new(McpErrorCode::Transport));
            }
            if matches!(
                &notification.notification,
                ServerNotification::ToolListChangedNotification(_)
            ) {
                state.catalog_stale = true;
            }
        }
    }
    Ok(())
}

const fn frame_error_code(error: framing::FrameError) -> McpErrorCode {
    match error {
        framing::FrameError::TooLarge => McpErrorCode::OutputBound,
        framing::FrameError::Deadline => McpErrorCode::Timeout,
        framing::FrameError::Io
        | framing::FrameError::Incomplete
        | framing::FrameError::Malformed => McpErrorCode::Transport,
    }
}

fn map_service_error(error: &ServiceError) -> McpError {
    match error {
        ServiceError::Timeout { .. } => McpError::new(McpErrorCode::Timeout),
        ServiceError::Cancelled { .. } => McpError::new(McpErrorCode::Cancellation),
        _ => McpError::new(McpErrorCode::Transport),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rmcp::{
        RoleClient,
        model::{ClientJsonRpcMessage, ClientRequest, PingRequest, RequestId},
        transport::Transport,
    };
    use tokio::io::duplex;

    use super::{BoundedStdioTransport, TransportShared};
    use crate::McpErrorCode;

    #[tokio::test]
    async fn blocked_writer_fails_at_the_frame_deadline() {
        let (_server_output, client_reader) = duplex(1);
        let (client_writer, _server_input) = duplex(1);
        let shared = TransportShared::new(1, 1);
        let mut transport = BoundedStdioTransport::new(
            client_reader,
            client_writer,
            shared,
            1_024,
            Duration::from_millis(25),
            crate::progress::ProgressRouter::new(1, 1),
        );
        let message = ClientJsonRpcMessage::request(
            ClientRequest::PingRequest(PingRequest::default()),
            RequestId::Number(1),
        );

        let error =
            <BoundedStdioTransport<_, _> as Transport<RoleClient>>::send(&mut transport, message)
                .await
                .unwrap_err();
        assert_eq!(error.code, McpErrorCode::Timeout);
    }
}

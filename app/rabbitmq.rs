pub use log::{error, trace};
pub use std::rc::Rc;

use crate::notifier;

#[derive(Debug, Clone)]
pub struct Client {
    async_runtime: Rc<tokio::runtime::Runtime>,
    conn: Rc<lapin::Connection>,
}
impl Client {
    pub fn open(addr: &str) -> Result<Self, notifier::NotifierError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        trace!("connecting to rabbitmq: {}", addr);
        let conn = runtime.block_on(async {
            lapin::Connection::connect(addr, lapin::ConnectionProperties::default())
                .await
                .map_err(|e| {
                    error!("failed to connect to rabbitmq: {}", e);
                    notifier::NotifierError::Unavailable(e.to_string())
                })
        })?;
        trace!("connected to rabbitmq with {:?}", conn.configuration());

        Ok(Self {
            async_runtime: Rc::new(runtime),
            conn: Rc::new(conn),
        })
    }
}

impl notifier::Notifier for Client {
    // to: queue name
    // message: message to send
    fn notify(&self, to: &str, message: &str) -> Result<(), notifier::NotifierError> {
        self.async_runtime.block_on(async {
            let chan = self.conn.create_channel().await.map_err(|e| {
                error!("failed to create channel: {}", e);
                notifier::NotifierError::Unavailable(e.to_string())
            })?;
            let _ = chan.basic_publish(
                "",
                to,
                lapin::options::BasicPublishOptions::default(),
                message.as_bytes(),
                lapin::BasicProperties::default(),
            );
            trace!("published: {} to {}", message, to);

            Ok(())
        })
    }
}

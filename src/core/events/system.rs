use super::*;
use crate::core::dom::{NodeId};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

pub type EventHandler = Box<dyn Fn(&Event) -> bool + Send + Sync>;
pub type EventCallback = Arc<dyn Fn(&Event) + Send + Sync>;

pub struct EventSystem {
    event_queue: Arc<RwLock<VecDeque<Event>>>,
    global_handlers: Arc<RwLock<HashMap<String, Vec<EventCallback>>>>,
    element_handlers: Arc<RwLock<HashMap<NodeId, HashMap<String, Vec<EventCallback>>>>>,
    event_sender: mpsc::UnboundedSender<Event>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<Event>>>,
}

impl EventSystem {
    pub fn new() -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        
        Self {
            event_queue: Arc::new(RwLock::new(VecDeque::new())),
            global_handlers: Arc::new(RwLock::new(HashMap::new())),
            element_handlers: Arc::new(RwLock::new(HashMap::new())),
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    pub async fn dispatch_event(&self, event: Event) {
        let event_type = self.get_event_type(&event);
        let mut propagate = true;

        if let Some(target_id) = self.get_event_target(&event) {
            propagate = self.handle_element_event(target_id, &event_type, &event).await;
        }

        if propagate {
            self.handle_global_event(&event_type, &event).await;
        }

        let mut queue = self.event_queue.write().await;
        queue.push_back(event);
    }

    pub async fn add_global_listener<F>(&self, event_type: &str, callback: F)
    where
        F: Fn(&Event) + Send + Sync + 'static,
    {
        let mut handlers = self.global_handlers.write().await;
        handlers
            .entry(event_type.to_string())
            .or_insert_with(Vec::new)
            .push(Arc::new(callback));
    }

    pub async fn add_element_listener<F>(&self, element_id: NodeId, event_type: &str, callback: F)
    where
        F: Fn(&Event) + Send + Sync + 'static,
    {
        let mut handlers = self.element_handlers.write().await;
        handlers
            .entry(element_id)
            .or_insert_with(HashMap::new)
            .entry(event_type.to_string())
            .or_insert_with(Vec::new)
            .push(Arc::new(callback));
    }

    pub async fn remove_element_listeners(&self, element_id: NodeId) {
        let mut handlers = self.element_handlers.write().await;
        handlers.remove(&element_id);
    }

    pub async fn process_events(&self) -> Vec<Event> {
        let mut queue = self.event_queue.write().await;
        let events: Vec<Event> = queue.drain(..).collect();
        events
    }

    pub fn send_event(&self, event: Event) -> Result<(), mpsc::error::SendError<Event>> {
        self.event_sender.send(event)
    }

    pub async fn poll_event(&self) -> Option<Event> {
        let mut receiver = self.event_receiver.write().await;
        receiver.try_recv().ok()
    }

    async fn handle_global_event(&self, event_type: &str, event: &Event) {
        let handlers = self.global_handlers.read().await;
        if let Some(callbacks) = handlers.get(event_type) {
            for callback in callbacks {
                callback(event);
            }
        }
    }

    async fn handle_element_event(&self, element_id: NodeId, event_type: &str, event: &Event) -> bool {
        let handlers = self.element_handlers.read().await;
        if let Some(element_handlers) = handlers.get(&element_id) {
            if let Some(callbacks) = element_handlers.get(event_type) {
                for callback in callbacks {
                    callback(event);
                }
                return false;
            }
        }
        true
    }

    fn get_event_type(&self, event: &Event) -> String {
        match event {
            Event::Mouse(mouse_event) => match mouse_event.event_type {
                MouseEventType::Down => "mousedown".to_string(),
                MouseEventType::Up => "mouseup".to_string(),
                MouseEventType::Move => "mousemove".to_string(),
                MouseEventType::Enter => "mouseenter".to_string(),
                MouseEventType::Leave => "mouseleave".to_string(),
                MouseEventType::Wheel => "wheel".to_string(),
            },
            Event::Keyboard(keyboard_event) => match keyboard_event.event_type {
                KeyboardEventType::Down => "keydown".to_string(),
                KeyboardEventType::Up => "keyup".to_string(),
                KeyboardEventType::Press => "keypress".to_string(),
            },
            Event::Touch(touch_event) => match touch_event.event_type {
                TouchEventType::Start => "touchstart".to_string(),
                TouchEventType::Move => "touchmove".to_string(),
                TouchEventType::End => "touchend".to_string(),
                TouchEventType::Cancel => "touchcancel".to_string(),
            },
            Event::Resize(_) => "resize".to_string(),
            Event::Scroll(_) => "scroll".to_string(),
            Event::Custom(custom_event) => custom_event.name.clone(),
        }
    }

    fn get_event_target(&self, _event: &Event) -> Option<NodeId> {
        None
    }
}

impl Default for EventSystem {
    fn default() -> Self {
        Self::new()
    }
}
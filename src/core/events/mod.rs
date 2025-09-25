pub mod system;

pub use system::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Mouse(MouseEvent),
    Keyboard(KeyboardEvent),
    Touch(TouchEvent),
    Resize(ResizeEvent),
    Scroll(ScrollEvent),
    Custom(CustomEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseEvent {
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
    pub event_type: MouseEventType,
    pub modifiers: KeyModifiers,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardEvent {
    pub key: String,
    pub code: String,
    pub event_type: KeyboardEventType,
    pub modifiers: KeyModifiers,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchEvent {
    pub touches: Vec<TouchPoint>,
    pub event_type: TouchEventType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchPoint {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResizeEvent {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollEvent {
    pub delta_x: f64,
    pub delta_y: f64,
    pub delta_z: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEvent {
    pub name: String,
    pub data: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MouseEventType {
    Down,
    Up,
    Move,
    Enter,
    Leave,
    Wheel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyboardEventType {
    Down,
    Up,
    Press,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TouchEventType {
    Start,
    Move,
    End,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

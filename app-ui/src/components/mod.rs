#![allow(dead_code, unused_imports)]
//! Shared UI components (shadcn-style kit under `ui/`).
//!
//! Install / refresh primitives with the rust-ui CLI from this crate root:
//!   ui add button input progress sonner badge card label separator textarea select -y

pub mod hooks;
pub mod ui;

pub use ui::badge::{Badge, BadgeSize, BadgeVariant};
pub use ui::button::{Button, ButtonSize, ButtonVariant};
pub use ui::card::{
  Card, CardAction, CardContent, CardDescription, CardFooter, CardHeader, CardItem, CardList,
  CardSize, CardTitle,
};
pub use ui::input::{Input, InputType};
pub use ui::label::Label;
pub use ui::progress::Progress;
pub use ui::select::{
  Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectOption, SelectPosition,
  SelectTrigger, SelectValue,
};
pub use ui::separator::{Separator, SeparatorOrientation};
pub use ui::sonner::{
  SonnerContainer, SonnerDirection, SonnerList, SonnerPosition, SonnerToaster, SonnerTrigger,
  ToastType,
};
pub use ui::switch::Switch;
pub use ui::textarea::Textarea;

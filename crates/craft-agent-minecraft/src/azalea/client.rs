//! Azalea 内部 Client 封装与底层辅助。
//!
//! `AzaleaBot` 已在 [`super::AzaleaBot`] 持有状态通道。本模块预留底层查询扩展点，
//! 当前感知统一通过 [`super::perception`] 与事件流（`BotEvent::State`）完成。

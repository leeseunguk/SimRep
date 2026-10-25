//! Shared, bounded Lua 5.1 runtime. No filesystem or process libraries.
use std::sync::atomic::{AtomicU64, Ordering};

pub use mlua;
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Value};

/// Create a pure Lua interpreter with 256 MiB and five billion instructions.
/// Input formats install only their own explicitly controlled host callbacks.
pub fn sandbox(label: &'static str) -> mlua::Result<Lua> {
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )?;
    lua.set_memory_limit(256 * 1024 * 1024)?;
    let budget = AtomicU64::new(5_000_000_000);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(10000),
        move |_, _| {
            if budget
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                    remaining.checked_sub(10000)
                })
                .is_err()
            {
                return Err(mlua::Error::RuntimeError(format!(
                    "{label} Lua instruction limit exceeded"
                )));
            }
            Ok(mlua::VmState::Continue)
        },
    )?;
    for name in [
        "dofile",
        "loadfile",
        "load",
        "loadstring",
        "require",
        "module",
        "getfenv",
        "setfenv",
        "collectgarbage",
        "newproxy",
    ] {
        lua.globals().set(name, Value::Nil)?;
    }
    Ok(lua)
}

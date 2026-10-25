//! Managed modpack updates, backups, overrides and staged upgrades.

mod managed;
mod overrides;
mod staged;
mod switch;
mod target;

pub use self::managed::update_managed_modpack;
pub(crate) use self::managed::*;
pub(crate) use self::overrides::*;
pub(crate) use self::staged::*;
pub use self::switch::switch_installed_file_version;
pub(crate) use self::switch::*;
pub use self::target::get_local_modpack_target;
pub(crate) use self::target::*;

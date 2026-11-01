use std::rc::Rc;

use floem::reactive::{RwSignal, Scope, SignalUpdate};
use goldenide_proxy::plugin::wasi::find_all_volts;
use goldenide_rpc::plugin::{VoltID, VoltInfo, VoltMetadata};
use indexmap::IndexMap;

use crate::window_tab::CommonData;

/// Metadata of a locally installed plugin (volt).
#[derive(Clone)]
pub struct InstalledVoltData {
    pub meta: RwSignal<VoltMetadata>,
}

/// Holds the plugins found in the local plugins directory.
///
/// There is no plugin marketplace: plugins are only loaded from disk.
#[derive(Clone, Debug)]
pub struct PluginData {
    pub installed: RwSignal<IndexMap<VoltID, InstalledVoltData>>,
    pub common: Rc<CommonData>,
}

impl PluginData {
    /// Creates the plugin data and scans the local plugin directories for
    /// plugins that only provide configuration (no wasm).
    pub fn new(cx: Scope, common: Rc<CommonData>) -> Self {
        let installed = cx.create_rw_signal(IndexMap::new());
        let plugin = Self { installed, common };

        {
            let plugin = plugin.clone();
            let extra_plugin_paths =
                plugin.common.window_common.extra_plugin_paths.clone();

            let send = floem::ext_event::create_ext_action(
                cx,
                move |volts: Vec<VoltMetadata>| {
                    for meta in volts {
                        plugin.volt_installed(&meta);
                    }
                },
            );
            std::thread::Builder::new()
                .name("FindAllVolts".to_owned())
                .spawn(move || {
                    let volts = find_all_volts(&extra_plugin_paths)
                        .into_iter()
                        .filter(|meta| meta.wasm.is_none())
                        .collect::<Vec<_>>();
                    send(volts);
                })
                .unwrap();
        }

        plugin
    }

    /// Registers (or refreshes) a plugin found on disk.
    pub fn volt_installed(&self, volt: &VoltMetadata) {
        let volt_id = volt.id();
        self.installed.update(|installed| {
            if let Some(existing) = installed.get(&volt_id) {
                existing.meta.set(volt.clone());
            } else {
                let data = InstalledVoltData {
                    meta: self.common.scope.create_rw_signal(volt.clone()),
                };
                installed.insert(volt_id, data);
            }
        });
    }

    /// Forgets a plugin that is no longer present on disk.
    pub fn volt_removed(&self, volt: &VoltInfo) {
        let id = volt.id();
        self.installed.update(|installed| {
            installed.swap_remove(&id);
        });
    }
}

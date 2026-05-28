pub mod error;
pub mod handlers;
pub mod modules;
pub mod rect;
pub mod resource;
pub mod state;
pub mod system_state;
pub mod utils;

pub use error::LibError;

#[macro_use(Serialize, Deserialize)]
extern crate serde;

#[macro_use(JsonSchema)]
extern crate schemars;

#[macro_use(TS)]
extern crate ts_rs;

extern crate num_enum;

#[cfg(feature = "gen-binds")]
#[test]
fn generate_schemas() {
    use state::{IconPack, Settings, TaskbarItems, Theme};

    fn write_schema<T>(path: &str)
    where
        T: schemars::JsonSchema,
    {
        let schema = schemars::schema_for!(T);
        std::fs::write(path, serde_json::to_string_pretty(&schema).unwrap()).unwrap();
    }

    std::fs::create_dir_all("./gen/schemas").unwrap();
    write_schema::<Settings>("./gen/schemas/settings.schema.json");

    write_schema::<TaskbarItems>("./gen/schemas/taskbar_items.schema.json");

    write_schema::<Theme>("./gen/schemas/theme.schema.json");
    write_schema::<IconPack>("./gen/schemas/icon_pack.schema.json");

    handlers::FuncEvent::generate_ts_file("./src/handlers/events.ts");
    handlers::FuncCommand::generate_ts_file("./src/handlers/commands.ts");
}

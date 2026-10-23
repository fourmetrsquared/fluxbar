//! Minimal FluxBar plugin.
//!
//! Build it with `cargo build -p fluxbar-plugin-hello`, install it with
//! `fluxbar plugin install examples/plugins/hello --no-build`, and add an item
//! named `hello` to your configuration.

use fluxbar_sdk::async_trait;
use fluxbar_sdk::{Plugin, PluginContext, PluginEvent, PluginOutput, run};
use std::f64::consts::TAU;

struct HelloPlugin {
    greeting: String,
    ticks: u64,
}

#[async_trait]
impl Plugin for HelloPlugin {
    async fn on_init(&mut self, context: &PluginContext) -> fluxbar_sdk::Result<()> {
        // Plugins receive their `[plugins.<name>.config]` table here.
        if let Some(greeting) = context.config_str("greeting") {
            self.greeting = greeting.to_string();
        }
        Ok(())
    }

    async fn on_event(
        &mut self,
        event: &PluginEvent,
        _context: &PluginContext,
    ) -> fluxbar_sdk::Result<PluginOutput> {
        let mut output = PluginOutput::new();
        match event.kind() {
            "tick" => return self.on_tick(_context).await,
            "build_finished" => {
                output.set_label("hello", "Build finished!");
            }
            other => {
                output.log(format!("received `{other}`"));
            }
        }
        Ok(output)
    }

    async fn on_tick(&mut self, _context: &PluginContext) -> fluxbar_sdk::Result<PluginOutput> {
        self.ticks += 1;
        let phase = (self.ticks as f64 * TAU / 20.0).sin() * 50.0 + 50.0;

        let mut output = PluginOutput::new();
        output
            .set_progress("hello", format!("{} #{}", self.greeting, self.ticks), phase)
            .set_graph(
                "hello-graph",
                (0..30)
                    .map(|i| ((i as f64 + self.ticks as f64) * TAU / 20.0).sin())
                    .collect(),
            );
        Ok(output)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> fluxbar_sdk::Result<()> {
    run(HelloPlugin {
        greeting: "Hello, FluxBar!".into(),
        ticks: 0,
    })
    .await
}

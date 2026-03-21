// Standalone binary — requires the `standalone` feature.
// Build with: cargo run -p sine_one --features standalone -- --output "Built-in Output"
fn main() {
    nih_plug::wrapper::standalone::nih_export_standalone::<sine_one::SineOne>();
}

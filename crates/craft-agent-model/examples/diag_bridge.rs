/// Test the adapter + bridge directly (no LLM involved)
use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
use std::time::Instant;

fn main() {
    println!("Connecting to MC mod...");
    let t0 = Instant::now();
    let adapter = MinecraftModAdapter::connect_with_vision("127.0.0.1", 25567, None)
        .expect("connect");
    println!("Connected in {:.2}s", t0.elapsed().as_secs_f64());

    println!("Querying state...");
    let t0 = Instant::now();
    match adapter.reload() {
        Ok(st) => {
            println!("State in {:.2}s", t0.elapsed().as_secs_f64());
            println!("  pos=({:.1f},{:.0f},{:.1f}) health={:.0f} hunger={}",
                st.position[0], st.position[1], st.position[2], st.health, st.hunger);
            println!("  held={} targeted={:?}", st.held_item, st.targeted_block.as_ref().map(|b| &b.id));
            println!("  inventory items: {}", st.inventory.iter().filter(|i| i.count > 0).count());
            println!("  nearby blocks: {}", st.nearby_blocks.len());
        }
        Err(e) => println!("Error: {e}"),
    }
}

fn main() {
    for p in bigame_core::benchmark::provider::all() {
        println!("{:20} {:?}", p.id(), p.availability());
    }
}

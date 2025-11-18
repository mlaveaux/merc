use merc_aterm::ATerm;
use merc_aterm::Symbol;
use merc_aterm::Term;

fn main() {
    let term = {
        let t = ATerm::constant(&Symbol::new("a", 0));
        t.arg(0)
    };

    // Have some side effect
    println!("Term: {:?}", term);
}
use merc_aterm::Protected;
use merc_aterm::ATermRef;
use merc_aterm::ATerm;
use merc_aterm::Symbol;
use merc_aterm::Term;

fn main() {
    let mut container: Protected<Vec<ATermRef<'static>>> = Protected::new(Vec::new());

    // Add stuff to the container to test soundness
    {
        let mut write = container.write();
        let t = write.protect(&ATerm::constant(&Symbol::new("a", 0)));
        write.push(t);
    }

    // This should not compile
    let t: ATermRef<'static> = {
        let read = container.read();
        read[0].copy()
    };
    println!("Term: {:?}", t);

    // This should not compile either
    let t: ATermRef<'static> = {
        let mut write = container.write();
        write.pop().unwrap()
    };
    println!("Term: {:?}", t);
}
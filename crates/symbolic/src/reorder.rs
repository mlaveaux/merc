use merc_utilities::MercError;
use mt_kahypar::Context;


pub fn reorder() -> Result<(), MercError> {


    let context = Context::builder().build()?;

    Ok(())
}

/// Represents a dependency graph between variables used in symbolic transition relations.
pub struct DependencyGraph {
    relations: Vec<Relation>,
}

/// A single relation in the dependency graph containing read and write
/// dependencies onto variables, given by their indices.
struct Relation{
    read_vars: Vec<usize>,
    write_vars: Vec<usize>,
}

/// Parses a dependency graph as output by
/// [lpreach](https://mcrl2.org/web/user_manual/tools/release/lpsreach.html) and
/// [pbessolvesymbolic](https://mcrl2.org/web/user_manual/tools/release/pbessolvesymbolic.html)
/// flag `--info`.
pub fn parse_compacted_dependency_graph(input: &str) -> DependencyGraph {
    let mut relations = Vec::new();

    for line in input.lines() {
        // Keep only pattern characters, ignoring indices/whitespace
        let pattern: Vec<char> = line
            .chars()
            .filter(|c| matches!(c, '+' | '-' | 'r' | 'w'))
            .collect();

        if pattern.is_empty() {
            continue;
        }

        let mut read_vars = Vec::new();
        let mut write_vars = Vec::new();

        for (col, ch) in pattern.into_iter().enumerate() {
            match ch {
                '+' => {
                    read_vars.push(col);
                    write_vars.push(col);
                }
                'r' => read_vars.push(col),
                'w' => write_vars.push(col),
                '-' => {}
                _ => {}
            }
        }

        relations.push(Relation { read_vars, write_vars });
    }

    DependencyGraph { relations }
}


#[cfg(test)]
mod tests {
    use crate::parse_compacted_dependency_graph;

    #[test]
    fn test_parse_abp_dependency_graph() {
        let input = "1 +w---------
2 ---+++-----
3 ------++---
4 --------++-
5 ------+w+w+
6 ---+ww--+w-
7 ---+++--+wr
8 +-----+w---
9 +rr+ww-----
10 +++---++---";

        let graph = parse_compacted_dependency_graph(input);

        assert_eq!(graph.relations.len(), 10);
    }
}
// The benchmark's graph: a synthetic social/product graph, deterministic per
// node count, in the shape the bench queries assume. One `Person` per node
// count unit (u%07d), one `Product` per ten persons (s%06d), and ~4.5
// relationships per node: `KNOWS` (symmetric friendships, two per person: one
// random long-range, one same-city) and `PURCHASED` (one per person).
//
// Everything both engines load comes from here: the CSV shards (zega's
// `mutation csv` blocks reference them, Neo4j's loader streams them over Bolt)
// and params.json, the seeded parameter list the throughput client replays
// against both engines.

use std::collections::HashSet;

use serde_json::json;

// ---------------------------------------------------------------- rng

/// splitmix64, as in zega-bench/src/scale.rs: the same graph for the same
/// node count, everywhere.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

// ---------------------------------------------------------------- the shape

const CITIES: &[&str] = &[
    "Lisbon", "Osaka", "Denver", "Nairobi", "Krakow", "Hobart", "Quito", "Tampere", "Gdańsk",
    "Valencia", "Busan", "Tucson", "Mombasa", "Porto", "Bergen", "Adelaide", "Oaxaca", "Tallinn",
    "Genoa", "Kigali",
];

const CATEGORIES: &[&str] = &[
    "books", "music", "games", "garden", "kitchen", "sports", "toys", "tools",
];

const FIRST: &[&str] = &[
    "Mara", "Jonas", "Priya", "Tomas", "Ines", "Kofi", "Lena", "Ravi", "Sofia", "Nils", "Aya",
    "Bruno", "Clara", "Dmitri", "Elin", "Farid", "Greta", "Hugo", "Iris", "Jamal", "Kira", "Lars",
    "Mina", "Otto",
];

const LAST: &[&str] = &[
    "Okafor", "Lindqvist", "Marchetti", "Novak", "Ferreira", "Haddad", "Kowalski", "Nguyen",
    "Petrov", "Quinn", "Rossi", "Santos", "Tanaka", "Umar", "Vasquez", "Weber", "Xu", "Yamamoto",
    "Zhang", "Ali", "Berg", "Costa", "Dubois", "Eze",
];

const ADJ: &[&str] = &[
    "Amber", "Brisk", "Copper", "Durable", "Easy", "Fresh", "Golden", "Handy", "Iron", "Jolly",
    "Keen", "Lucky", "Mighty", "Noble", "Orange", "Prime", "Quick", "Rapid", "Solid", "Tidy",
    "Ultra", "Vivid", "Warm", "Zesty",
];

const NOUN: &[&str] = &[
    "Kettle", "Lantern", "Backpack", "Canteen", "Compass", "Blanket", "Stove", "Tent", "Hammer",
    "Wrench", "Basket", "Cradle", "Tablet", "Speaker", "Headlamp", "Rucksack", "Thermos",
    "Griddle", "Planter", "Toolkit", "Carrier", "Flashlight", "Radio", "Corkboard",
];

pub const PARAM_COUNT: usize = 4096;

fn person_name(rng: &mut Rng, i: u64) -> String {
    format!(
        "{} {}{}",
        FIRST[rng.below(FIRST.len() as u64) as usize],
        LAST[rng.below(LAST.len() as u64) as usize],
        i % 100
    )
}

fn product_name(rng: &mut Rng, i: u64) -> String {
    format!(
        "{} {} {}",
        ADJ[rng.below(ADJ.len() as u64) as usize],
        NOUN[rng.below(NOUN.len() as u64) as usize],
        100 + i % 900
    )
}

/// Stay under zega#127's 2,000,000-byte import ceiling with margin.
const SHARD_LIMIT: usize = 1_900_000;

/// CSV rows into shards of at most SHARD_LIMIT bytes, header per shard.
/// Field texts here never contain , " or \n, so no quoting is needed.
fn shards(dir: &std::path::Path, stem: &str, header: &str, rows: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    let mut body = String::with_capacity(SHARD_LIMIT / 4);
    let flush = |names: &mut Vec<String>, body: &mut String| {
        if body.is_empty() {
            return;
        }
        let name = format!("{stem}-{:03}.csv", names.len() + 1);
        let mut text = String::with_capacity(header.len() + 1 + body.len());
        text.push_str(header);
        text.push('\n');
        text.push_str(body);
        std::fs::write(dir.join(&name), text).expect("write shard");
        names.push(name);
        body.clear();
    };
    for row in rows {
        if header.len() + 1 + body.len() + row.len() + 1 > SHARD_LIMIT {
            flush(&mut names, &mut body);
        }
        body.push_str(row);
        body.push('\n');
    }
    flush(&mut names, &mut body);
    names
}

struct Graph {
    persons: Vec<String>,
    products: Vec<String>,
    knows: Vec<String>,
    purchased: Vec<String>,
}

fn build_graph(n: u64) -> Graph {
    let mut rng = Rng(0x0b4c_0000_0000_0000 ^ n);
    let persons_n = n;
    let products_n = (n / 10).max(1);

    let mut persons = Vec::with_capacity(persons_n as usize);
    for i in 0..persons_n {
        let city = CITIES[(i % CITIES.len() as u64) as usize];
        persons.push(format!(
            "u{i:07},{},u{}@example.social,{},{}",
            person_name(&mut rng, i),
            i,
            city,
            1998 + rng.below(28)
        ));
    }

    let mut products = Vec::with_capacity(products_n as usize);
    for i in 0..products_n {
        products.push(format!(
            "s{i:06},{},{},{}",
            product_name(&mut rng, i),
            CATEGORIES[(i % CATEGORIES.len() as u64) as usize],
            500 + rng.below(200_000),
        ));
    }

    // Friendships: an undirected edge set, stored as two directed KNOWS rows.
    // One random long-range edge per person keeps the diameter logarithmic (a
    // real social graph is small-world); one same-city edge adds locality.
    // Out-degree lands near four per person; ~5 relationships per node in all.
    let mut edges: HashSet<(u64, u64)> = HashSet::with_capacity(persons_n as usize * 2);
    let cities = CITIES.len() as u64;
    for i in 0..persons_n {
        let mut j1 = rng.below(persons_n);
        if j1 == i {
            j1 = (j1 + 1) % persons_n;
        }
        edges.insert(if i < j1 { (i, j1) } else { (j1, i) });
        let c = i % cities;
        let in_city = (persons_n - 1 - c) / cities + 1;
        let mut j2 = c + cities * rng.below(in_city);
        if j2 == i {
            j2 = (i + cities) % persons_n;
        }
        if j2 != j1 {
            edges.insert(if i < j2 { (i, j2) } else { (j2, i) });
        }
    }
    let mut knows = Vec::with_capacity(edges.len() * 2);
    for &(a, b) in &edges {
        knows.push(format!("u{a:07},u{b:07}"));
        knows.push(format!("u{b:07},u{a:07}"));
    }

    let mut purchased = Vec::with_capacity(persons_n as usize);
    for i in 0..persons_n {
        purchased.push(format!("u{i:07},s{:06}", rng.below(products_n)));
    }

    Graph {
        persons,
        products,
        knows,
        purchased,
    }
}

// ---------------------------------------------------------------- gen

const SCHEMA_AND_MUTATIONS: &str = "\
// Synthetic social/product graph for the zega vs Neo4j bench. Generated by
// `bench gen`; deterministic per node count. Do not edit by hand.
schema {
  type Person {
    id: String
    name: String
    email: String
    city: String
    joined: Int
    knows: KNOWS -> Person[]
    purchases: PURCHASED -> Product[]
  }
  type Product {
    sku: String
    name: String
    category: String
    price: Int
    buyers: PURCHASED <- Person[]
  }
}

unique {
  Person { id }
  Product { sku }
}
";

pub fn gen(args: &[String]) {
    let n: u64 = args
        .first()
        .and_then(|s| s.replace('_', "").parse().ok())
        .unwrap_or_else(|| usage());
    let dir = std::path::PathBuf::from(args.get(1).unwrap_or_else(|| usage()));
    std::fs::create_dir_all(&dir).expect("data dir");

    let g = build_graph(n);
    let person_shards = shards(&dir, "persons", "id,name,email,city,joined", &g.persons);
    let product_shards = shards(&dir, "products", "sku,name,category,price", &g.products);
    let knows_shards = shards(&dir, "knows", "from,to", &g.knows);
    let purchased_shards = shards(&dir, "purchased", "from,to", &g.purchased);

    let mut zql = String::from(SCHEMA_AND_MUTATIONS);
    zql.push('\n');
    for name in &person_shards {
        zql.push_str(&format!(
            "mutation csv [\"./{name}\"] {{\n  Person(id: $id && name: $name && email: $email && city: $city && joined: $joined)\n}}\n\n"
        ));
    }
    for name in &product_shards {
        zql.push_str(&format!(
            "mutation csv [\"./{name}\"] {{\n  Product(sku: $sku && name: $name && category: $category && price: $price)\n}}\n\n"
        ));
    }
    for (field, target, target_key, list) in [
        ("knows", "Person", "id", &knows_shards),
        ("purchases", "Product", "sku", &purchased_shards),
    ] {
        for name in list {
            zql.push_str(&format!(
                "mutation csv [\"./{name}\"] {{\n  Person(id: $from) {{ {field} -> link {target}({target_key}: $to) }}\n}}\n\n"
            ));
        }
    }
    std::fs::write(dir.join("schema.zql"), zql.trim_end().to_string() + "\n").expect("schema.zql");

    // The seeded parameter list both engines replay. Drawn from a stream
    // separate from the graph's so params are reproducible even if the graph
    // shape is tweaked.
    let persons_n = n;
    let mut rng = Rng(0x0be4_9e9e_0000_0000 ^ n);
    let pick = |rng: &mut Rng| rng.below(persons_n);
    let mut lookup = Vec::with_capacity(PARAM_COUNT);
    let mut onehop = Vec::with_capacity(PARAM_COUNT);
    let mut twohop = Vec::with_capacity(PARAM_COUNT);
    let mut filtered = Vec::with_capacity(PARAM_COUNT);
    for _ in 0..PARAM_COUNT {
        lookup.push(pick(&mut rng));
        onehop.push(pick(&mut rng));
        twohop.push(pick(&mut rng));
        filtered.push(json!([pick(&mut rng), rng.below(CATEGORIES.len() as u64)]));
    }
    let mut path = Vec::with_capacity(PARAM_COUNT / 2);
    for _ in 0..PARAM_COUNT / 2 {
        let a = pick(&mut rng);
        let mut b = pick(&mut rng);
        if a == b {
            b = (b + 1) % persons_n;
        }
        path.push(json!([a, b]));
    }
    let params = json!({
        "lookup": lookup,
        "onehop": onehop,
        "twohop": twohop,
        "filtered": filtered,
        "path": path,
        "categories": CATEGORIES,
    });
    std::fs::write(dir.join("params.json"), params.to_string()).expect("params.json");

    let csv_bytes: u64 = std::fs::read_dir(&dir)
        .expect("read dir")
        .map(|e| e.expect("entry").metadata().expect("meta").len())
        .sum();
    let meta = json!({
        "nodes": n + n / 10,
        "persons": n,
        "products": n / 10,
        "knows": g.knows.len(),
        "purchased": g.purchased.len(),
        "rels": g.knows.len() + g.purchased.len(),
        "csv_bytes": csv_bytes,
        "shards": person_shards.len() + product_shards.len() + knows_shards.len()
            + purchased_shards.len(),
    });
    std::fs::write(dir.join("meta.json"), meta.to_string()).expect("meta.json");
    println!("{meta}");
}

pub fn categories() -> &'static [&'static str] {
    CATEGORIES
}

pub fn usage() -> ! {
    eprintln!(
        "usage:\n  bench gen <nodes> <dir>\n  bench load-zega --dataset <dir> --data <data-dir>\n  bench load-neo4j --dataset <dir> --url <host:7687> --password <pw> [--clear]\n  bench parity --zega-url <url> --neo4j-url <host:7687> --password <pw> --dataset <dir> [--samples N]\n  bench run --engine zega|neo4j|neo4j-http --url <url> --password <pw> --dataset <dir> --query <kind> --conc <n> --seconds <n> [--warmup s] [--write-base n] [--tag s]"
    );
    std::process::exit(2)
}

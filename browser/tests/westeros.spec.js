import { readFile, readdir } from 'node:fs/promises';
import { test, expect } from './offline.js';

// The Westeros sample (APS 22 "Universe graphs") in the shipped wasm engine:
// the fan questions the graph is built for, asked in the filter grammar from
// docs/relationships.md (has / !have / in / same / hops). Expected answers
// are hand-set from the books and cross-checked against the AWOIAF source
// pages during the build (see ATTRIBUTION.md and westeros-sample.mjs); the
// sample's own files are read straight off disk, like flights.spec.js does.
// A query naming one node by a unique field returns that node alone, so
// ask() normalizes every result to a list.
const SAMPLE = 'samples/westeros.zql';

async function loadSample(page) {
  const zql = await readFile(SAMPLE, 'utf8');
  const files = (await readdir('samples')).filter((name) => name.startsWith('westeros-') && name.endsWith('.csv'));
  const sources = {};
  for (const name of files) sources[`./samples/${name}`] = await readFile(`samples/${name}`, 'utf8');
  return page.evaluate(async ({ zql, sources }) => {
    const { default: init, ZegaWasm } = await import('/pkg/zega_wasm.js');
    await init();
    const db = new ZegaWasm();
    try {
      db.apply_with_sources(zql, JSON.stringify(sources));
      const ask = (body) => [JSON.parse(db.run_with_sources(zql, `query ${body}`, '{}'))].flat();
      return {
        heldWinterfell: ask('{ Location(name: "Winterfell") { name heldBy <- Character { name &from &to } } }'),
        killedByLannister: ask('{ Character(has killedBy in memberOf(name: "House Lannister")) { name } }'),
        swornToTully: ask('{ House(has swornTo(name: "House Tully")) { name } }'),
        starksAlive: ask('{ Character(alive = true && has memberOf(name: "House Stark")) { name } }'),
        starksDead: ask('{ Character(alive = false && has memberOf(name: "House Stark")) { name } }'),
        castlesNorth: ask('{ Location(kind = "castle" && has region(name: "The North")) { name } }'),
        battlesRiverlands: ask('{ Event(kind = "battle" && has happenedAt in region(name: "The Riverlands")) { name year } }'),
        daenerysTravels: ask('{ Character(name: "Daenerys Targaryen") { tookPartIn -> Event { name year happenedAt -> Location { name } } } }'),
        seatInOwnRegion: ask('{ House(has region && has seat in same region) { name } }'),
        houseless: ask('{ Character(!have memberOf) { name } }'),
        twoFatherHops: ask('{ Character(has father exactly 2 hops(name = "Rickard Stark")) { name } }'),
        tyrionSiblings: ask('{ Character(name: "Tyrion Lannister") { sibling -> Character { name } } }'),
        eddardChildren: ask('{ Character(name: "Eddard Stark") { fatherOf <- Character { name } } }'),
        margaerySpouses: ask('{ Character(name: "Margaery Tyrell") { spouse -> Character { name } } }'),
        klHolders: ask('{ Location(name: "King\'s Landing") { name heldBy <- Character { name &from &to } } }'),
        recentEvents: ask('{ Event(year >= 298) { name } }'),
        redWedding: ask('{ Event(name: "Red Wedding") { name year happenedAt -> Location { name } participants <- Character { name } } }'),
        blackwater: ask('{ Event(name: "Battle of the Blackwater") { participants <- Character { name } } }'),
        noVassals: ask('{ House(!have vassals) { name } }'),
        lannisterSeats: ask('{ House(has swornTo(name: "House Lannister")) { name seat -> Location { name } } }'),
        northLocations: ask('{ Region(name: "The North") { name locations <- Location { name } } }'),
        sansaLineage: ask('{ Character(name: "Sansa Stark") { name father -> Character { name } mother -> Character { name } } }'),
        jonKilled: ask('{ Character(name: "Jon Snow") { killed <- Character { name } } }'),
        seatLinks: ask('{ House(has seat(name = "The Twins") || has seat(name = "Pinkmaiden") || has seat(name = "Hammerhorn")) { name seat -> Location { name } } }'),
      };
    } finally {
      db.free();
    }
  }, { zql, sources });
}

test('the Westeros sample answers its fan questions', async ({ page }) => {
  await page.goto('/');
  const answers = await loadSample(page);
  const list = (value) => [value].flat();
  const names = (rows) => list(rows).map((row) => row.name).sort();

  // Who has held Winterfell: Eddard from the end of the rebellion until his
  // death, Robb until the Red Wedding, then Ramsay. (AWOIAF's own page dates
  // Eddard's death to 299 AC; the dataset keeps the book-timeline convention
  // of 298 used across the events.)
  expect(list(answers.heldWinterfell[0].heldBy).sort((a, b) => a.from - b.from)).toEqual([
    { name: 'Eddard Stark', from: 283, to: 298 },
    { name: 'Robb Stark', from: 298, to: 299 },
    { name: 'Ramsay Bolton', from: 299, to: 300 },
  ]);
  // Every character killed by a man of House Lannister: Jaime killed Aerys,
  // Tyrion killed Tywin.
  expect(names(answers.killedByLannister)).toEqual(['Aerys II Targaryen', 'Tywin Lannister']);
  // Houses sworn to Tully.
  expect(names(answers.swornToTully)).toEqual([
    'House Blackwood', 'House Bracken', 'House Darry', 'House Frey',
    'House Mallister', "House Piper", "House Vance of Wayfarer's Rest", 'House Whent',
  ]);
  // Starks with no recorded death in the source: the five children and Jon,
  // plus household members whose pages record no death (Benjen is missing
  // beyond the Wall in the books; Hodor, Osha and Old Nan are Stark household).
  expect(names(answers.starksAlive)).toEqual([
    'Arya Stark', 'Benjen Stark', 'Bran Stark', 'Hodor', 'Jon Snow',
    'Lyarra Stark', 'Old Nan', 'Osha', 'Rickon Stark', 'Sansa Stark',
  ]);
  expect(names(answers.starksDead)).toEqual([
    'Brandon Stark', 'Eddard Stark', 'Lyanna Stark', 'Rickard Stark', 'Robb Stark',
  ]);
  // Castles in the North.
  expect(names(answers.castlesNorth)).toEqual([
    'Castle Black', 'Cerwyn', 'Deepwood Motte', 'Eastwatch-by-the-Sea',
    'Greywater Watch', 'Hornwood', 'Karhold', 'Last Hearth', 'Moat Cailin',
    'Oldcastle', 'The Dreadfort', 'The Shadow Tower', "Torrhen's Square",
    'Widow\'s Watch', 'Winterfell',
  ]);
  // Battles in the Riverlands, with their AC years: the Trident and the Bells
  // in Robert's Rebellion, then the war of the five kings (the Whispering
  // Wood's page dates it in the older "AL" style — Years after Aegon's
  // Landing, the same conquest-era count the dataset calls AC).
  expect(list(answers.battlesRiverlands).sort((a, b) => String(a.year).localeCompare(String(b.year)))).toEqual([
    { name: 'Battle of the Bells', year: 283 },
    { name: 'Battle of the Trident', year: 283 },
    { name: 'Battle of Riverrun', year: 298 },
    { name: 'Battle of the Whispering Wood', year: 298 },
    { name: 'Battle of the Camps', year: 299 },
    { name: 'Battle of the Fords', year: 299 },
    { name: 'Battle of the Green Fork', year: 299 },
    { name: 'Battle of the Ruby Ford', year: 299 },
  ]);
  // Daenerys's travels: Astapor, then the siege of Meereen, with where each
  // happened.
  expect(answers.daenerysTravels[0].tookPartIn).toEqual([
    { name: 'Siege of Meereen', year: 299, happenedAt: { name: 'Meereen' } },
    { name: 'Sack of Astapor', year: 299, happenedAt: { name: 'Astapor' } },
  ]);
  // Houses whose seat sits in their own region (the `same` join): the great
  // houses and most vassals; far more than the handful whose seats are lost
  // or outside their region.
  expect(names(answers.seatInOwnRegion).length).toBeGreaterThan(60);
  for (const house of ['House Stark', 'House Lannister', 'House Tyrell', 'House Martell', 'House Tully']) {
    expect(names(answers.seatInOwnRegion), house).toContain(house);
  }
  // Characters with no recorded house allegiance: sellswords, free folk,
  // men of the Watch sworn to no house.
  expect(names(answers.houseless).length).toBeGreaterThan(100);
  for (const who of ['Petyr Baelish', 'Tormund Giantsbane', 'Melisandre', 'Vargo Hoat', 'High Sparrow']) {
    expect(names(answers.houseless), who).toContain(who);
  }
  // Rickard Stark's grandchildren, exactly two father-hops down.
  expect(names(answers.twoFatherHops)).toEqual([
    'Arya Stark', 'Bran Stark', 'Rickon Stark', 'Robb Stark', 'Sansa Stark',
  ]);
  // Tyrion's siblings; Eddard's children; Margaery's husbands.
  expect(names(answers.tyrionSiblings[0].sibling)).toEqual(['Cersei Lannister', 'Jaime Lannister']);
  expect(names(answers.eddardChildren[0].fatherOf)).toEqual([
    'Arya Stark', 'Bran Stark', 'Rickon Stark', 'Robb Stark', 'Sansa Stark',
  ]);
  expect(names(answers.margaerySpouses[0].spouse)).toEqual([
    'Joffrey Baratheon', 'Renly Baratheon', 'Tommen Baratheon',
  ]);
  // Who held King's Landing: Aerys until the Sack, then the Baratheons.
  expect(list(answers.klHolders[0].heldBy).sort((a, b) => a.from - b.from)).toEqual([
    { name: 'Aerys II Targaryen', from: 262, to: 283 },
    { name: 'Robert Baratheon', from: 283, to: 298 },
    { name: 'Joffrey Baratheon', from: 298, to: 300 },
    { name: 'Tommen Baratheon', from: 300, to: null },
  ]);
  // Events of 298 AC and later, including the Whispering Wood (its page dates
  // it 298AL) and the siege of Meereen.
  expect(names(answers.recentEvents)).toEqual([
    'Battle of Castle Black', 'Battle of Deepwood Motte', 'Battle of Oxcross',
    'Battle of Riverrun', 'Battle of the Blackwater', 'Battle of the Camps',
    'Battle of the Fords', 'Battle of the Golden Tooth', 'Battle of the Green Fork',
    'Battle of the Ruby Ford', 'Battle of the Shield Islands', 'Battle of the Whispering Wood',
    'Fall of Moat Cailin', 'Red Wedding', 'Sack of Astapor', 'Sack of Winterfell',
    'Siege of Meereen',
  ]);
  // The Red Wedding: 299 AC at the Twins.
  expect(answers.redWedding[0].year).toBe(299);
  expect(answers.redWedding[0].happenedAt.name).toBe('The Twins');
  expect(names(answers.redWedding[0].participants)).toEqual([
    'Robb Stark', 'Roose Bolton', 'Walder Frey', 'Walder Rivers',
  ]);
  // The Battle of the Blackwater, both command.
  expect(names(answers.blackwater[0].participants)).toEqual([
    'Davos Seaworth', 'Garlan Tyrell', 'Imry Florent', 'Mace Tyrell',
    'Randyll Tarly', 'Salladhor Saan', 'Sandor Clegane', 'Stannis Baratheon',
    'Tyrion Lannister', 'Tywin Lannister',
  ]);
  // Houses nobody is sworn to: every house but the eight overlords.
  expect(names(answers.noVassals).length).toBeGreaterThan(85);
  expect(names(answers.noVassals)).not.toContain('House Stark');
  expect(names(answers.noVassals)).not.toContain('House Lannister');
  expect(names(answers.noVassals)).toContain('House Clegane');
  // Seats of houses sworn to the Lannisters.
  expect(answers.lannisterSeats.filter((h) => h.seat).map((h) => [h.name, h.seat.name]).sort()).toEqual([
    ['House Banefort', 'Banefort'], ['House Brax', 'Hornvale'], ['House Crakehall', 'Crakehall'],
    ['House Farman', 'Faircastle'], ['House Marbrand', 'Ashemark'], ['House Prester', 'Feastfires'],
    ['House Reyne', 'Castamere'], ['House Spicer', 'Castamere'], ['House Swyft', 'Cornfield'],
    ['House Tarbeck', 'Tarbeck Hall'], ['House Westerling', 'The Crag'],
  ]);
  // The North holds its locations, the Wolfswood among them. (Wintertown
  // would sit outside Winterfell's walls, but the archive never captured its
  // source page, so it is dropped from the dataset.)
  expect(names(answers.northLocations[0].locations)).toEqual([
    'Barrowton', 'Bear Island', 'Castle Black', 'Cerwyn', 'Deepwood Motte',
    'Eastwatch-by-the-Sea', 'Greywater Watch', 'Hornwood', 'Karhold', 'Last Hearth',
    'Moat Cailin', "Mole's Town", 'Oldcastle', 'Queenscrown', 'Ramsgate',
    'Sea Dragon Point', 'Skagos', 'The Dreadfort', 'The Gift', 'The Neck',
    'The Shadow Tower', "Torrhen's Square", 'White Harbor', "Widow's Watch", 'Winterfell',
    'Wolfswood',
  ]);
  // Seats whose infobox names differ from the dataset's page titles: the
  // Twins gained their "The" on the wiki, Pinkmaiden Castle is Pinkmaiden,
  // and Hammerhorn was added as a place so House Goodbrother keeps a seat.
  expect(answers.seatLinks.map((h) => [h.name, h.seat.name]).sort()).toEqual([
    ['House Frey', 'The Twins'], ['House Goodbrother', 'Hammerhorn'], ['House Piper', 'Pinkmaiden'],
  ]);
  // Sansa's parents.
  expect(answers.sansaLineage[0].father.name).toBe('Eddard Stark');
  expect(answers.sansaLineage[0].mother.name).toBe('Catelyn Stark');
  // Jon Snow killed Qhorin Halfhand.
  expect(names(answers.jonKilled[0].killed)).toEqual(['Qhorin Halfhand']);
});

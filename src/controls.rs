//! The optional modern controls: a click uses what it hits (the Avatar
//! walks there first if he has to), a key uses what is at hand, and the
//! object under the pointer lights up.

use crate::effects::{Pathfind, PROC_WALK};
use crate::game::{Game, AVATAR};
use crate::map::WorldBox;
use crate::vm::Kind;

const EVENT_USE: u32 = 1;
/// How far the Avatar can reach, between the footprints.
const REACH: i32 = 96;
const REACH_HEIGHT: i32 = 64;
/// Shape of the key ring (key K uses it, as in the original).
pub const KEYRING: u32 = 79;

/// What the player has told the Avatar to do and he is still about:
/// walking to a spot or a thing, climbing there, going for an enemy.
/// Not saved; a loaded game starts without orders.
#[derive(Default)]
pub struct Orders {
    /// The height of the spot the player sent the Avatar to with a click:
    /// he has only arrived on that level (a click below an edge is not
    /// reached by standing above it).
    pub(crate) walk_level: Option<i32>,
    /// That spot is on something small (a pillar): only there exactly is he there.
    pub(crate) walk_exact: bool,
    /// Modern controls: the item the Avatar walks to in order to use it.
    pub(crate) approaching: Option<u16>,
    /// Walking by click to a higher place (see jump.rs).
    pub(crate) climb_goal: Option<crate::jump::ClimbGoal>,
    /// Fighting by clicking: the enemy the Avatar goes for.
    pub(crate) click_fight: Option<ClickFight>,
    /// Modern controls: shift switches between walking and running.
    pub(crate) run_toggle: bool,
}

/// Fighting by clicking, as in Diablo: the enemy that was clicked.
#[derive(Clone, Copy)]
pub struct ClickFight {
    pub target: u16,
    /// The mouse button is still down: keep striking.
    pub held: bool,
    /// A blow has been struck since the click.
    swung: bool,
    /// Walks towards the enemy since the last blow.
    misses: u8,
}

/// Gap between the footprints at which a blow reaches (as for the creatures).
const STRIKE_GAP: i32 = 20;

impl Game {
    /// Does using this item do anything?
    pub fn usable(&mut self, id: u16) -> bool {
        let Some(it) = self.map().get(id).copied() else { return false };
        if id != AVATAR && self.fights.states.get(&id).is_some_and(|s| s.dead) {
            return true; // a body can be searched
        }
        // the quarters of the Great Pentacle: their script does something
        // only for a whole small pentacle (frame 4); they are floor to walk on
        const PENTACLE: u32 = 124;
        if it.shape == PENTACLE && it.frame != 4 {
            return false;
        }
        let class = self.usecode_class(&it);
        self.scripts.usecode.event(class, EVENT_USE).is_some()
    }

    /// Someone the Avatar would fight.
    pub fn hostile(&mut self, id: u16) -> bool {
        let is_actor = id != AVATAR && self.map().get(id).is_some_and(|it| self.can_fight(it) && it.in_world());
        if !is_actor {
            return false;
        }
        let (s, me) = (self.state(id), self.state(AVATAR));
        let fights_him = s.in_combat && s.target == AVATAR;
        // the townsfolk only once they fight him (see run_combat); so also
        // creatures a script made, such as the golem of the spell, which a
        // click gives orders to (its use) as long as it does not fight him
        // and someone with something to say (a click talks to them, as
        // Arcadion in the Obsidian Fortress, whose kind would fight him)
        let (dead, hates_him) = (s.dead, s.enemy_alignment & me.alignment != 0);
        let peaceful = (s.alignment & crate::combat::TOWNSFOLK != 0 || self.fights.script_made.contains(&id) || self.usable(id)) && !fights_him;
        !dead && hates_him && !peaceful
    }

    fn at_hand(&self, id: u16) -> bool {
        let (Some(item), avatar) = (self.map().get(id).copied(), self.avatar()) else { return false };
        // what lies in a container or is worn is at hand
        !item.in_world() || (self.box_gap(&avatar, &item) <= REACH && (item.z - avatar.z).abs() <= REACH_HEIGHT)
            // (option: all over a pentacle he stands near, to light the candles)
            || self.pentagram_reach(item.x, item.y, item.z)
    }

    /// A click on an item: use it, walking up to it first if it is too
    /// far away.
    pub fn use_or_approach(&mut self, id: u16) {
        self.cancel_approach();
        if self.at_hand(id) || !self.usable(id) {
            self.interact(id, true);
        } else if self.player_in_control() {
            self.add_native(Kind::Pathfind(Pathfind::new(AVATAR, id, (0, 0))), AVATAR, PROC_WALK);
            self.orders.approaching = Some(id);
        }
    }

    /// A click on the ground: walk there. False if the pointer is not on
    /// ground one can stand on.
    pub fn walk_to_pointer(&mut self, x: i32, y: i32) -> bool {
        self.cancel_approach();
        if !self.player_in_control() {
            return false;
        }
        let Some(((tx, ty, tz), small)) = self.ground_spot(x, y) else { return false };
        self.start_walk(tx, ty);
        self.orders.walk_level = Some(tz);
        self.orders.walk_exact = small;
        // higher up than a step: the ledges on the way are climbed
        // (as high: on the way there may be a step down, and up again)
        if tz >= self.avatar().z - 8 {
            self.orders.climb_goal = Some(crate::jump::ClimbGoal::new(tx, ty, tz));
        }
        true
    }

    /// The Avatar walks to a world position.
    pub fn start_walk(&mut self, tx: i32, ty: i32) {
        self.orders.walk_level = None;
        self.add_native(Kind::Pathfind(Pathfind::new(AVATAR, 0, (tx, ty))), AVATAR, PROC_WALK);
        // 0: only walking, nothing to use on arrival
        self.orders.approaching = Some(0);
    }

    /// Is the Avatar on the way somewhere he was sent?
    pub fn walking(&self) -> bool {
        self.scripts.procs.iter().any(|p| p.item == AVATAR && p.ptype == PROC_WALK && !p.terminated)
    }

    /// The player steers again: the Avatar stops walking to what was clicked.
    pub fn cancel_approach(&mut self) {
        self.orders.click_fight = None;
        self.orders.walk_level = None;
        self.orders.walk_exact = false;
        if let Some(g) = self.orders.climb_goal.take() {
            if g.nudging {
                self.inputs.remove(&AVATAR);
            }
        }
        if let Some(m) = self.actors.get_mut(&AVATAR) {
            m.may_drop = false;
        }
        if self.orders.approaching.take().is_some() {
            self.kill_processes(AVATAR, PROC_WALK);
            self.inputs.remove(&AVATAR);
            self.paths.remove(&AVATAR);
        }
    }

    /// The button held on the ground (hold to follow): the Avatar follows
    /// the pointer from now on. A walk to a spot clicked before ends at
    /// once instead of being finished first. False while he is not the
    /// player's to move (a scene, a conversation).
    pub fn start_following(&mut self) -> bool {
        if !self.player_may_order() {
            return false;
        }
        self.cancel_approach();
        true
    }

    /// Once the Avatar has arrived at what was clicked, he uses it.
    pub fn tick_approach(&mut self) {
        let Some(id) = self.orders.approaching else { return };
        if self.scripts.procs.iter().any(|p| p.item == AVATAR && p.ptype == PROC_WALK && !p.terminated) {
            return; // still on the way
        }
        self.orders.approaching = None;
        if id == 0 {
            return; // just a walk
        }
        // the walk ends at talking distance; no use if it never got near
        let near = match (self.map().get(id).copied(), self.avatar()) {
            (Some(item), avatar) => self.box_gap(&avatar, &item) <= 2 * REACH,
            _ => false,
        };
        if near {
            self.interact(id, true);
        }
    }

    /// The usable thing closest to the Avatar, in front of him rather
    /// than behind.
    pub fn use_nearest(&mut self) -> Option<u16> {
        const XF: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
        const YF: [i32; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];
        if !self.player_may_order() {
            return None;
        }
        let avatar = self.avatar();
        let dir = self.mover(AVATAR).dir as usize & 7;
        let centre = |g: &Game, it: &crate::map::Item| {
            let b = g.data.infos.get(it.shape as usize).map_or(WorldBox { x: it.x, y: it.y, z: it.z, xd: 0, yd: 0, zd: 0 }, |si| WorldBox::of(it, si));
            (b.x - b.xd / 2, b.y - b.yd / 2)
        };
        let (ax, ay) = centre(self, &avatar);
        let near: Vec<crate::map::Item> = self.map().items_near(avatar.x, avatar.y, 320).into_iter().map(|i| self.map().items[i]).collect();
        let mut best: Option<(i32, u16)> = None;
        for it in near {
            if it.id == 0 || it.id == AVATAR || !it.in_world() || (it.z - avatar.z).abs() > REACH_HEIGHT {
                continue;
            }
            let gap = self.box_gap(&avatar, &it);
            if gap > REACH - 16 || !self.usable(it.id) {
                continue;
            }
            let (cx, cy) = centre(self, &it);
            // the closest; what is behind him has to be clearly closer
            let ahead = (cx - ax) * XF[dir] + (cy - ay) * YF[dir];
            let score = gap + if ahead > 0 { 0 } else { 48 };
            if best.is_none_or(|b| score < b.0) {
                best = Some((score, it.id));
            }
        }
        let (_, id) = best?;
        // a walk the player sent him on ends here
        self.cancel_approach();
        self.interact(id, true);
        Some(id)
    }

    /// Remembers what the pointer is on, to light it up. Returns true if
    /// that has changed.
    pub fn set_hover(&mut self, pointer: Option<(i32, i32)>) -> bool {
        // Not while the Avatar is on the move: the world slides along under
        // the pointer and one thing after the other would flash up.
        let still = self.orders.approaching.is_none() && !self.mover(AVATAR).busy();
        // With the weapon drawn he moves all the time (strikes, stance):
        // what is under the pointer stays found then instead of flashing.
        // (walking, running, advancing, retreating still count as moving)
        let fighting = self.state(AVATAR).in_combat && self.orders.approaching.is_none() && ![0, 1, 8, 9].contains(&self.mover(AVATAR).last_anim());
        let hover = match pointer {
            Some((x, y)) if (still || fighting) && self.player_in_control() && self.ui.drag.is_none() => {
                self.pick(x, y).filter(|&id| self.noteworthy(id))
            }
            _ => None,
        };
        let changed = hover != self.ui.hover;
        if changed {
            self.ui.hover_name = hover.and_then(|id| self.puzzle_spot_name(id)).or_else(|| hover.filter(|_| self.options.hover_names != 0).and_then(|id| self.look_name(id).or_else(|| self.unnamed_thing(id))));
        }
        self.ui.hover = hover;
        changed
    }

    /// The keys and buttons of the controls in use, for the options menu.
    pub fn control_help(&self) -> Vec<(&'static str, &'static str)> {
        let mut help = self.control_help_base();
        if self.options.modern_controls {
            // shift switches running on and off; a click on the ground can walk there
            let (shift, ground) = self.tr([
                (("Shift", "Rennen an, aus"), ("Klick Boden", "Dorthin laufen")),
                (("Maj", "Courir oui, non"), ("Clic au sol", "Y aller")),
                (("Shift", "Running on, off"), ("Click ground", "Walk there")),
            ]);
            help[0].1 = self.tr(["Gehen", "Marcher", "Walk"]);
            help.insert(1, shift);
            if self.options.click_walk {
                help.insert(3, ground);
                if self.options.hold_walk {
                    help.insert(4, self.tr([
                        ("Boden halten", "Dem Zeiger folgen"),
                        ("Sol, tenir", "Suivre le pointeur"),
                        ("Hold on ground", "Follow the pointer"),
                    ]));
                }
            }
        }
        if self.options.modern_controls {
            // fighting by clicking: a click goes for the enemy, ctrl strikes on the spot
            if let Some(i) = help.iter().position(|h| ["im Kampf:", "au combat :", "in combat:"].contains(&h.0)) {
                let (click, ctrl) = self.tr([
                    ("Klick: hin und schlagen,", ("Strg-Klick", "Schlag auf der Stelle")),
                    ("clic : y aller, frapper,", ("Ctrl-clic", "frapper sur place")),
                    ("click: go and strike,", ("Ctrl-click", "strike on the spot")),
                ]);
                help[i].1 = click;
                // after the lines that continue the combat line
                let mut j = i + 1;
                while j < help.len() && help[j].0.is_empty() {
                    j += 1;
                }
                help.insert(j, ctrl);
            }
        }
        // zoom, and turning the view if switched on
        help.push(self.tr([("Plus Minus", "Ansicht gr\u{94}\u{e1}er, kleiner"), ("Plus, Moins", "Zoom"), ("Plus, Minus", "Zoom in, out")]));
        if self.options.turn_view {
            help.push(self.tr([("Komma Punkt", "Ansicht drehen"), ("Virgule, Point", "Tourner la vue"), ("Comma, Period", "Turn the view")]));
        }
        help.push(self.tr([("L", "Beleuchtung an, aus"), ("L", "Eclairage oui, non"), ("L", "Lighting on, off")]));
        help.push(self.tr([("G", "Grafikfilter: aus, xBR, CRT"), ("G", "Filtre : aucun, xBR, CRT"), ("G", "Graphics filter: off, xBR, CRT")]));
        help.push(self.tr([("M", "Minikarte an, aus"), ("M", "Mini-carte oui, non"), ("M", "Minimap on, off")]));
        if self.options.journal {
            help.push(self.tr([("T", "Tagebuch"), ("T", "Journal"), ("T", "Quest journal")]));
        }
        if self.options.spell_notes {
            help.push(self.tr([("N", "Zauberbuch"), ("N", "Grimoire"), ("N", "Spell book")]));
        }
        help.push(self.tr([
            ("1 - 5", "Trank: gelb, rot, orange, lila, schwarz"),
            ("1 - 5", "Potion : jaune, rouge, orange, violette, noire"),
            ("1 - 5", "Potion: yellow, red, orange, purple, black"),
        ]));
        help
    }

    fn control_help_base(&self) -> Vec<(&'static str, &'static str)> {
        match (self.options.modern_controls, self.language) {
            (true, 'G') => vec![
                ("WASD, Pfeile", "Gehen (Shift: rennen)"),
                ("Linksklick", "Benutzen, hingehen"),
                ("Rechtsklick", "Aufheben, ansehen; Boden: springen"),
                ("Ziehen", "Gegenstand bewegen"),
                ("", "Stapel: Menge w\u{84}hlen, Shift: alles"),
                ("E", "N\u{84}chstes benutzen"),
                ("H (halten)", "Benutzbares zeigen"),
                ("Leertaste", "Springen, klettern"),
                ("Tab", "Waffe ziehen"),
                ("im Kampf:", "Links schlagen,"),
                ("", "rechts: Waffe weg, Q Tritt, B Block"),
                ("", "rechts doppelt: treten"),
                ("I, K", "Rucksack, Schl\u{81}sselring"),
                ("Esc", "Fenster zu, Men\u{81}"),
                ("F5, F9", "Schnell sichern, laden"),
            ],
            (true, 'F') => vec![
                ("ZQSD, fl\u{8a}ches", "Marcher (Maj : courir)"),
                ("Clic gauche", "Utiliser, y aller"),
                ("Clic droit", "Ramasser, regarder; sol: sauter"),
                ("Glisser", "D\u{82}placer un objet"),
                ("", "pile : choisir, Maj : tout"),
                ("E", "Utiliser le plus proche"),
                ("H (tenir)", "Montrer les objets"),
                ("Espace", "Sauter, grimper"),
                ("Tab", "D\u{82}gainer"),
                ("au combat :", "gauche frapper,"),
                ("", "droit rengainer, Q pied, B parer"),
                ("", "double droit : coup de pied"),
                ("I, K", "Sac, trousseau de cl\u{82}s"),
                ("Echap", "Fermer, menu"),
                ("F5, F9", "Sauvegarde rapide"),
            ],
            (true, _) => vec![
                ("WASD, arrows", "Walk (Shift: run)"),
                ("Left click", "Use, walk there"),
                ("Right click", "Pick up, look at; ground: jump"),
                ("Drag", "Move an item"),
                ("", "stack: pick amount, Shift: all"),
                ("E", "Use the nearest"),
                ("H (hold)", "Show usable things"),
                ("Space", "Jump, climb"),
                ("Tab", "Draw the weapon"),
                ("in combat:", "left strikes,"),
                ("", "right: weapon away, Q kicks, B blocks"),
                ("", "right double click: kick"),
                ("I, K", "Backpack, key ring"),
                ("Esc", "Close windows, menu"),
                ("F5, F9", "Quick save, load"),
            ],
            (false, 'G') => vec![
                ("WASD, Pfeile", "Gehen (Shift: rennen)"),
                ("Rechte Taste", "Zum Zeiger laufen"),
                ("Linksklick", "Ansehen"),
                ("Shift+Rechts", "In den Rucksack nehmen"),
                ("Doppelklick", "Benutzen"),
                ("Ziehen", "Gegenstand bewegen"),
                ("", "Stapel: Menge w\u{84}hlen, Shift: alles"),
                ("Leertaste", "Springen, klettern"),
                ("Tab", "Waffe ziehen"),
                ("im Kampf:", "Klick, F schlagen,"),
                ("", "Q treten, B blocken"),
                ("", "rechts doppelt: treten"),
                ("I, C", "Rucksack, Fenster zu"),
                ("K", "Schl\u{81}sselring"),
                ("Esc", "Men\u{81}"),
                ("F5, F9", "Schnell sichern, laden"),
            ],
            (false, 'F') => vec![
                ("ZQSD, fl\u{8a}ches", "Marcher (Maj : courir)"),
                ("Bouton droit", "Aller vers le pointeur"),
                ("Clic gauche", "Regarder"),
                ("Maj+droit", "Mettre dans le sac"),
                ("Double clic", "Utiliser"),
                ("Glisser", "D\u{82}placer un objet"),
                ("", "pile : choisir, Maj : tout"),
                ("Espace", "Sauter, grimper"),
                ("Tab", "D\u{82}gainer"),
                ("au combat :", "clic, F frapper,"),
                ("", "Q coup de pied, B parer"),
                ("", "double droit : coup de pied"),
                ("I, C", "Sac, fermer"),
                ("K", "Trousseau de cl\u{82}s"),
                ("Echap", "Menu"),
                ("F5, F9", "Sauvegarde rapide"),
            ],
            (false, _) => vec![
                ("WASD, arrows", "Walk (Shift: run)"),
                ("Right button", "Walk to the pointer"),
                ("Left click", "Look at"),
                ("Shift+right", "Put in the backpack"),
                ("Double click", "Use"),
                ("Drag", "Move an item"),
                ("", "stack: pick amount, Shift: all"),
                ("Space", "Jump, climb"),
                ("Tab", "Draw the weapon"),
                ("in combat:", "click, F strikes,"),
                ("", "Q kicks, B blocks"),
                ("", "right double click: kick"),
                ("I, C", "Backpack, close"),
                ("K", "Key ring"),
                ("Esc", "Menu"),
                ("F5, F9", "Quick save, load"),
            ],
        }
    }

    // ---- fighting by clicking ----------------------------------------------

    /// Someone a click with the weapon drawn goes for: an enemy, or anyone
    /// fighting the Avatar.
    /// Uses the key ring the Avatar carries (also in a bag in the
    /// backpack), as if it were double-clicked: it asks for the lock. Says
    /// so if he has none.
    pub fn use_keyring(&mut self) -> bool {
        if !self.player_in_control() || self.state(AVATAR).dead {
            return false;
        }
        let found = self.map().items.iter().find(|it| !it.gone && it.shape == KEYRING && it.parent != 0 && self.root_container(it.id) == AVATAR).map(|it| it.id);
        match found {
            Some(id) => {
                self.interact(id, true);
                true
            }
            None => {
                // (the game fonts use the DOS character set)
                let text = self.tr(["Kein Schl\u{81}sselring", "Pas de trousseau de cl\u{82}s", "No key ring"]);
                self.notify(text);
                false
            }
        }
    }

    /// Drinks a potion the Avatar carries (also in bags in the backpack), by
    /// its colour: `n` 0 = yellow (a little health), 1 = red (full health),
    /// 2 = orange (mana), 3 = purple (protection), 4 = black (invisible).
    /// False if he has none of them.
    pub fn quick_potion(&mut self, n: usize) -> bool {
        use crate::potions::{POTION, QUICK_FRAMES as FRAMES};
        if n >= FRAMES.len() || !self.player_in_control() || self.state(AVATAR).dead {
            return false;
        }
        let found = self
            .map()
            .items
            .iter()
            .find(|it| !it.gone && it.shape == POTION && it.frame == FRAMES[n] && it.parent != 0 && self.root_container(it.id) == AVATAR)
            .map(|it| it.id);
        match found {
            Some(id) => {
                self.interact(id, true);
                true
            }
            None => {
                let text = match (self.language, n) {
                    ('G', 0) => "Kein gelber Trank",
                    ('G', 1) => "Kein roter Trank",
                    ('G', 2) => "Kein oranger Trank",
                    ('G', 3) => "Kein lila Trank",
                    ('G', _) => "Kein schwarzer Trank",
                    ('F', 0) => "Pas de potion jaune",
                    ('F', 1) => "Pas de potion rouge",
                    ('F', 2) => "Pas de potion orange",
                    ('F', 3) => "Pas de potion violette",
                    ('F', _) => "Pas de potion noire",
                    (_, 0) => "No yellow potion",
                    (_, 1) => "No red potion",
                    (_, 2) => "No orange potion",
                    (_, 3) => "No purple potion",
                    (_, _) => "No black potion",
                };
                self.notify(text);
                false
            }
        }
    }

    /// Can one stand on this (the ground, and posts, railings, crates: a
    /// place to jump to), rather than only look at it?
    pub fn is_ground(&self, id: u16) -> bool {
        self.map().get(id).filter(|it| !it.is_actor()).and_then(|it| self.data.infos.get(it.shape as usize)).is_some_and(|si| si.land)
    }

    pub fn attackable(&mut self, id: u16) -> bool {
        if self.hostile(id) {
            return true;
        }
        let is_actor = id != AVATAR && self.map().get(id).is_some_and(|it| self.can_fight(it) && it.in_world());
        let s = self.state(id);
        // (also someone still fighting whose foe has fallen: Malchir after
        // the daemon he called up went down, who hates nobody by his kind)
        let target_gone = s.target == 0 || self.fights.states.get(&s.target).is_some_and(|t| t.dead);
        // (and a creature that hates him but fights someone else: the
        // skeleton fighting the golem; not one that takes orders by a
        // click, as the golem itself)
        let (hates_him, townsfolk) = (s.enemy_alignment & self.state(AVATAR).alignment != 0, s.alignment & crate::combat::TOWNSFOLK != 0);
        let foe_elsewhere = hates_him && !townsfolk && !self.usable(id);
        is_actor && !s.dead && s.in_combat && (s.target == AVATAR || target_gone || foe_elsewhere)
    }

    /// The enemy a click at a view position goes for: its health bar, what
    /// is drawn there, or someone right behind a flame or the like drawn
    /// over him (Malchir in his ring of blue fire). `anyone`: any living
    /// person, as with the weapon drawn.
    pub fn foe_at(&mut self, x: i32, y: i32, anyone: bool) -> Option<u16> {
        let ok = |g: &mut Game, id: u16| g.attackable(id) || (anyone && g.living_person(id));
        if let Some(id) = self.bar_at(x, y).filter(|&id| ok(self, id)) {
            return Some(id);
        }
        let top = self.pick(x, y)?;
        if ok(self, top) {
            return Some(top);
        }
        let see_through = self.map().get(top).is_some_and(|it| !it.is_actor() && self.data.infos.get(it.shape as usize).is_some_and(|si| !si.solid));
        if !see_through {
            return None;
        }
        self.pick_world(x, y, top).filter(|&id| ok(self, id))
    }

    /// An enemy whose health bar (or the space from it down to its feet)
    /// is at a view position: so that even creatures one can hardly see
    /// can be clicked.
    pub fn bar_at(&self, x: i32, y: i32) -> Option<u16> {
        let y = self.unflip_y(y);
        self.ui.bar_areas.iter().find(|&&(_, x0, y0, x1, y1)| x >= x0 && x <= x1 && y >= y0 && y <= y1).map(|a| a.0)
    }

    /// The attack key: the Avatar turns to the nearest enemy close by,
    /// steps up to it and strikes (drawing his weapon first if need be);
    /// with nobody near he strikes straight ahead.
    pub fn attack_nearest(&mut self) {
        if !self.player_may_order() || self.state(AVATAR).dead {
            return;
        }
        const NEAR: i32 = 256;
        let av = self.avatar();
        let candidates: Vec<u16> = self
            .map()
            .items
            .iter()
            .filter(|it| self.can_fight(it) && it.id != AVATAR && !it.dead && !it.gone && it.in_world())
            .filter(|it| (it.x - av.x).abs() <= NEAR + 128 && (it.y - av.y).abs() <= NEAR + 128 && (it.z - av.z).abs() < 48)
            .map(|it| it.id)
            .collect();
        let mut best: Option<(i32, u16)> = None;
        for id in candidates {
            if !self.attackable(id) {
                continue;
            }
            let Some(t) = self.map().get(id).copied() else { continue };
            let gap = self.box_gap(&av, &t);
            if gap <= NEAR && best.is_none_or(|(g, _)| gap < g) {
                best = Some((gap, id));
            }
        }
        if !self.fighting() {
            self.cancel_approach();
            self.set_in_combat(AVATAR, true);
        }
        match best {
            Some((_, id)) => self.fight_target(id),
            None => {
                self.cancel_approach();
                self.avatar_attack(None, false)
            }
        }
    }

    /// Fighting by clicking (modern controls): a click on an enemy makes
    /// the Avatar go for it, drawing his weapon first if need be; with the
    /// weapon drawn a click elsewhere walks there. False if fighting by
    /// clicking does not apply.
    pub fn combat_click(&mut self, x: i32, y: i32) -> bool {
        if !self.options.modern_controls || !self.player_may_order() {
            return false;
        }
        if !self.fighting() {
            let Some(id) = self.foe_at(x, y, false) else { return false };
            // stop walking, draw the weapon, go for the enemy
            self.cancel_approach();
            self.set_in_combat(AVATAR, true);
            self.fight_target(id);
            return true;
        }
        // with the weapon drawn anyone can be attacked, friendly or not
        match self.foe_at(x, y, true) {
            Some(id) => self.fight_target(id),
            None => {
                self.walk_to_pointer(x, y);
            }
        }
        true
    }

    /// Someone other than the Avatar who is alive.
    pub fn living_person(&mut self, id: u16) -> bool {
        let is_actor = id != AVATAR && self.map().get(id).is_some_and(|it| self.can_fight(it) && it.in_world());
        is_actor && !self.state(id).dead
    }

    /// Goes for an enemy (the button is taken as held until released).
    pub fn fight_target(&mut self, id: u16) {
        self.cancel_approach();
        self.orders.click_fight = Some(ClickFight { target: id, held: true, swung: false, misses: 0 });
        self.tick_click_fight();
    }

    /// The mouse button is up: one more blow at most.
    pub fn combat_release(&mut self) {
        if let Some(f) = &mut self.orders.click_fight {
            f.held = false;
        }
    }

    /// One tick of fighting by clicking: close in on the enemy, strike
    /// when within reach.
    pub fn tick_click_fight(&mut self) {
        let Some(mut f) = self.orders.click_fight else { return };
        let target = self.map().get(f.target).copied().filter(|t| t.in_world() && !self.state(f.target).dead);
        let done = !self.fighting() || (f.swung && !f.held) || f.misses > 4;
        let Some(t) = target.filter(|_| !done) else {
            self.orders.click_fight = None;
            return;
        };
        let av = self.avatar();
        let (dx, dy) = (t.x - av.x, t.y - av.y);
        if self.box_gap(&av, &t) <= STRIKE_GAP && (t.z - av.z).abs() < 48 {
            // within reach: stop walking and strike
            if self.walking() {
                self.kill_processes(AVATAR, PROC_WALK);
                self.inputs.remove(&AVATAR);
                self.paths.remove(&AVATAR);
                self.orders.approaching = None;
            }
            if !self.mover(AVATAR).busy() {
                self.avatar_attack(Some(crate::intrinsics::world_dir(dx, dy) as u8), false);
                if self.mover(AVATAR).busy() {
                    f.swung = true;
                    f.misses = 0;
                }
            }
        } else if !self.walking() && !self.mover(AVATAR).busy() {
            // walk up to it; again if it got away (a few times at most)
            f.misses += 1;
            let mut pf = Pathfind::new(AVATAR, f.target, (0, 0));
            pf.touch = true;
            self.add_native(Kind::Pathfind(pf), AVATAR, PROC_WALK);
            // 0: walking only, nothing is used on arrival
            self.orders.approaching = Some(0);
        }
        self.orders.click_fight = Some(f);
    }
}

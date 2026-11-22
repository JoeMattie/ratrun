#!/usr/bin/env python3
"""Convert the owner's per-floor MIDI stems into compact Rust song tables.

Standard library only. This is a *build-time data* tool: it reads the stems,
cleans them up into something a small realtime sequencer can play, and writes
one `src/music/<floor>.rs` per song. Nothing here is loadable at runtime --
the game only ever sees the generated Rust.

    python3 tools/midi2music.py <midi-root> src/music [--only 3] [--roll]

`<midi-root>` is the directory holding the song folders `1` .. `6`, one folder
per song, each holding one `.mid` per stem. The mapping from song number to
floor lives in `SONGS` below and is the single place to change if the owner
swaps two songs around.

Pipeline (each stage is a function so it can be read on its own):

  1. `parse_midi`          -- format-0/1 `.mid` reader (notes + tempo map).
  2. `song_from_stems`     -- resolve the wobbling tempo map to one constant
                              BPM, detect the meter, quantise onsets to a 16th
                              grid, snap durations, drop ghost notes.
  3. `assign_roles`        -- classify each stem into a role by stem name,
                              register and statistics (top voice for lead,
                              lowest for bass, chords for pad, ...).
  4. `shape_clusters`      -- merge duplicates, octave smear, cap polyphony.
  5. `find_loop`           -- find a clean whole-bar loop that still contains
                              the song's sections, and slice to it.
  6. `mix_gains`           -- estimate each role's energy and normalise the
                              per-role gains to the background-music mix target.
  7. `emit_rust`           -- write the tables (u8 fields, delta times) plus a
                              text piano roll for a human sanity check.

The generated tables use one note tuple per event:

    (delta_steps, midi, gate_steps)

`delta_steps` is the wait *in 16th steps* since the previous event of the same
part (0 for a simultaneous note, e.g. the rest of a chord), `midi` is the MIDI
note or General MIDI percussion note, and `gate_steps` is how many grid steps
the note is held. A `midi` of 0 means "rest": advance time, sound nothing (only
needed for a gap longer than 255 steps).
"""

from __future__ import annotations

import argparse
import glob
import math
import os
import struct
import sys

# --------------------------------------------------------------------------
# Song table: the ONE place the song -> floor mapping lives.
# --------------------------------------------------------------------------

# (song folder, rust module, Theme, human title)
SONGS = [
    ("1", "sewer", "Sewer", "Silent Return"),
    ("2", "kitchen", "Kitchen", "The Great Dash"),
    ("3", "laundry", "Laundry", "A Minor Waltz"),
    ("4", "lab", "Lab", "Platen Pace"),
    ("5", "printworks", "Printworks", "B-C-D# Warning"),
    ("6", "flatbed", "Flatbed", "Rat on the Flatbed"),
]

# Per-song instrument assignment and timbre. The mix and the *material* make
# most of the difference between the six floors; these knobs make the rest:
# where the lead sits in the spectrum, how dark the bass is, how wide the pad
# is detuned, how the bus is driven. Written per song on purpose -- the six
# floors are meant to be recognisable from each other.
STYLES = {
    # Sewer: dark and muffled, everything filtered down under the drip.
    "1": dict(
        drive=1.16, master=1.0, bass_tri=0.62, bass_lp=520.0, lead_duty=0.30,
        lead_lp=1900.0, pad_detune=0.13, pad_lp=1200.0, arp_lp=2600.0,
        stab_duty=0.38, stab_lp=1500.0, kick_hz=48.0,
    ),
    # Kitchen: bright and plucky, the dash floor.
    "2": dict(
        drive=1.24, master=1.0, bass_tri=0.45, bass_lp=900.0, lead_duty=0.34,
        lead_lp=3400.0, pad_detune=0.10, pad_lp=2100.0, arp_lp=4200.0,
        stab_duty=0.42, stab_lp=2400.0, kick_hz=58.0,
    ),
    # Laundry: the strings waltz. Soft, slow pad, no bite on the lead.
    "3": dict(
        drive=1.08, master=1.0, bass_tri=0.75, bass_lp=680.0, lead_duty=0.44,
        lead_lp=2400.0, pad_detune=0.16, pad_lp=2000.0, arp_lp=2800.0,
        stab_duty=0.40, stab_lp=1700.0, kick_hz=52.0,
    ),
    # Lab: the press. Square lead, hard arp, the tightest bus.
    "4": dict(
        drive=1.32, master=1.0, bass_tri=0.40, bass_lp=780.0, lead_duty=0.50,
        lead_lp=2900.0, pad_detune=0.08, pad_lp=1600.0, arp_lp=2400.0,
        stab_duty=0.50, stab_lp=2100.0, kick_hz=62.0,
    ),
    # Printworks: brassy and wide, the three-note warning.
    "5": dict(
        drive=1.20, master=1.0, bass_tri=0.50, bass_lp=640.0, lead_duty=0.36,
        lead_lp=2700.0, pad_detune=0.20, pad_lp=1500.0, arp_lp=3400.0,
        stab_duty=0.44, stab_lp=2000.0, kick_hz=44.0,
    ),
    # Flatbed: driving, wide pad, the ride home.
    "6": dict(
        drive=1.28, master=1.0, bass_tri=0.52, bass_lp=860.0, lead_duty=0.32,
        lead_lp=4000.0, pad_detune=0.07, pad_lp=1900.0, arp_lp=3800.0,
        stab_duty=0.36, stab_lp=2200.0, kick_hz=60.0,
    ),
}

# Roles, in the fixed order the generated table uses. Must match
# `crate::music::Role` in src/music/mod.rs.
ROLES = ["kick", "snare", "hat", "perc", "bass", "pad", "arp", "lead", "stab", "fx"]
R = {name: i for i, name in enumerate(ROLES)}

# Per-role polyphony caps (simultaneous notes in one grid step).
CAPS = {
    "kick": 1,
    "snare": 1,
    "hat": 1,
    "perc": 2,
    "bass": 1,
    "pad": 4,
    "arp": 4,
    "lead": 2,
    "stab": 3,
    "fx": 2,
}

# Envelope and timbre of the runtime voices, mirrored from
# `music::synth` so the mix can be predicted offline. `attack_ms` is a linear
# attack, `decay_ms` a one-pole decay (0 = the note is gated by its length, i.e.
# it sustains), and `timbre_rms` the RMS of the raw waveform.
ATTACK_MS = {
    "kick": 0.5,
    "snare": 0.5,
    "hat": 0.2,
    "perc": 0.6,
    "bass": 6.0,
    "pad": 90.0,
    "arp": 3.0,
    "lead": 12.0,
    "stab": 8.0,
    "fx": 4.0,
}
DECAY_MS = {
    "kick": 90.0,
    "snare": 70.0,
    "hat": 26.0,
    "perc": 45.0,
    "bass": 0.0,
    "pad": 0.0,
    "arp": 140.0,
    "lead": 0.0,
    "stab": 240.0,
    "fx": 300.0,
}
TIMBRE_RMS = {
    "kick": 0.46,
    "snare": 0.38,
    "hat": 0.34,
    "perc": 0.33,
    "bass": 0.55,
    "pad": 0.50,
    "arp": 0.44,
    "lead": 0.54,
    "stab": 0.50,
    "fx": 0.34,
}
RELEASE_MS = 45.0

# Layer gate per intensity tier -- mirrors `music::synth::LAYER_GAIN`. Used here
# only to predict the RMS each tier lands on (the real table is in Rust).
# Calm is bass + pad + light percussion; tier 1 adds the drums and the
# arpeggio, tier 2 the lead, tier 3 everything plus the boss drive.
# No role ever loses gain as the tier rises, so the mix can only get fuller.
LAYER_GAIN = {
    "kick": [0.70, 0.95, 1.00, 1.05],
    "snare": [0.00, 0.90, 1.00, 1.05],
    "hat": [0.65, 1.00, 1.00, 1.00],
    "perc": [0.00, 0.85, 1.00, 1.05],
    "bass": [1.00, 1.00, 1.00, 1.05],
    "pad": [1.00, 1.00, 1.00, 1.00],
    "arp": [0.00, 0.95, 1.00, 1.00],
    "lead": [0.00, 0.00, 1.00, 1.10],
    "stab": [0.00, 0.00, 0.55, 1.05],
    "fx": [0.00, 0.00, 0.40, 1.00],
}

# Whole-mix gain per tier, on top of the layer gates. A tier always sounds a
# superset of the tier below at the same or a higher gain, so "rising intensity
# is fuller" is a property of the table, not a hope about the material.
TIER_GAIN = [1.00, 1.06, 1.13, 1.22]

# --------------------------------------------------------------------------
# 1. MIDI parsing
# --------------------------------------------------------------------------

NOTE_ON = 0x90
NOTE_OFF = 0x80


class RawNote:
    """One sounding note, in musical time (ticks)."""

    __slots__ = ("tick", "dur", "pitch", "vel", "ch")

    def __init__(self, tick, dur, pitch, vel, ch):
        self.tick = tick
        self.dur = dur
        self.pitch = pitch
        self.vel = vel
        self.ch = ch

    def __repr__(self):
        return f"<{self.pitch}@{self.tick}+{self.dur} v{self.vel} ch{self.ch}>"


def _vlq(buf, i):
    value = 0
    while True:
        byte = buf[i]
        i += 1
        value = (value << 7) | (byte & 0x7F)
        if byte < 0x80:
            return value, i


def parse_midi(path):
    """Return (division, notes, tempo_us_per_quarter). Treats the file as data.

    A note cut off without a note-off (a truncated or sloppy export) is closed
    at the end of its track, so no note is ever silently dropped.
    """
    try:
        with open(path, "rb") as fh:
            buf = fh.read()
    except OSError as exc:
        raise SystemExit(f"midi2music: cannot read {path}: {exc}") from None
    if buf[:4] != b"MThd":
        raise SystemExit(f"midi2music: {path} is not a MIDI file")
    _fmt, tracks, division = struct.unpack(">HHH", buf[8:14])
    i = 14
    notes = []
    tempos = []
    for _ in range(tracks):
        if buf[i : i + 4] != b"MTrk":
            raise ValueError(f"{path}: bad track chunk")
        length = struct.unpack(">I", buf[i + 4 : i + 8])[0]
        j = i + 8
        end = j + length
        tick = 0
        running = 0
        open_notes = {}
        while j < end:
            delta, j = _vlq(buf, j)
            tick += delta
            status = buf[j]
            if status == 0xFF:
                kind = buf[j + 1]
                size, k = _vlq(buf, j + 2)
                data = buf[k : k + size]
                j = k + size
                if kind == 0x51 and len(data) >= 3:
                    tempos.append(int.from_bytes(data, "big"))
            elif status in (0xF0, 0xF7):
                size, k = _vlq(buf, j + 1)
                j = k + size
            else:
                if status & 0x80:
                    running = status
                    j += 1
                else:
                    status = running
                hi = status & 0xF0
                ch = status & 0x0F
                if hi in (0xC0, 0xD0):
                    j += 1
                else:
                    a, b = buf[j], buf[j + 1]
                    j += 2
                    if hi == NOTE_ON and b > 0:
                        # A key repeating without a note-off replaces the old
                        # one rather than piling up two notes on one pitch.
                        if (ch, a) in open_notes:
                            t0, v = open_notes.pop((ch, a))
                            notes.append(RawNote(t0, max(tick - t0, 1), a, v, ch))
                        open_notes[(ch, a)] = (tick, b)
                    elif hi == NOTE_OFF or (hi == NOTE_ON and b == 0):
                        if (ch, a) in open_notes:
                            t0, v = open_notes.pop((ch, a))
                            notes.append(RawNote(t0, max(tick - t0, 1), a, v, ch))
        for (ch, pitch), (t0, v) in sorted(open_notes.items()):
            notes.append(RawNote(t0, max(tick - t0, 1), pitch, v, ch))
        i = end
    notes.sort(key=lambda n: (n.tick, n.pitch))
    return division, notes, tempos


# --------------------------------------------------------------------------
# 2. Tempo, meter, quantisation
# --------------------------------------------------------------------------

def resolve_bpm(tempos):
    """The wobbling tempo map (+/-0.2 BPM automated drift) as one constant BPM.

    The median of every tempo event in the song is the tempo the arrangement
    was written at; the wobble is a renderer artefact of the generator. Musical
    positions do not depend on it at all (a tick is a tick), so this only picks
    the seconds-per-step the sequencer clocks out.
    """
    if not tempos:
        return 120
    bpms = sorted(60_000_000.0 / t for t in tempos if t > 0)
    median = bpms[len(bpms) // 2]
    return int(round(median))


def detect_beats_per_bar(notes, division):
    """3 or 4, from where the arrangement repeats.

    Autocorrelating the 16th-note onset strength at a lag of three beats and of
    four beats asks the only question that matters here: does the material line
    up with itself three beats later or four? In 3/4 the three-beat lag wins
    (the waltz scores 0.905 against 0.861); in 4/4 the four-beat lag does, on
    every one of the six songs.
    """
    if not notes:
        return 4
    end = max(n.tick + n.dur for n in notes)
    steps = int(end / division * 4) + 1
    if steps < 48:
        return 4
    grid = [0.0] * steps
    for n in notes:
        idx = int(round(n.tick / division * 4))
        if 0 <= idx < steps:
            grid[idx] += n.vel / 127.0

    def autocorr(lag_beats):
        lag = lag_beats * 4
        if lag <= 0 or lag >= steps:
            return 0.0
        a = grid[: steps - lag]
        b = grid[lag:]
        num = sum(x * y for x, y in zip(a, b))
        den = math.sqrt(sum(x * x for x in a) * sum(y * y for y in b))
        return num / den if den > 0 else 0.0

    three = autocorr(3)
    four = autocorr(4)
    return 3 if three > four * 1.01 else 4


def quantise(notes, division, steps_per_bar, ghost_vel, ghost_frac):
    """Snap onsets and lengths to the 16th grid; drop ghost notes.

    `ghost_vel` is the velocity under which a note shorter than `ghost_frac`
    of the grid is a smear rather than a note (the generator leaves a few of
    these on nearly every stem), and `ghost_vel` alone drops near-silent
    notes whenever they fall.
    """
    grid = division / 4.0  # ticks per 16th
    out = []
    dropped = 0
    for n in notes:
        if n.vel <= 8:
            dropped += 1
            continue
        step = int(round(n.tick / grid))
        gate = int(round(n.dur / grid))
        if gate < 1:
            # Shorter than a 16th: keep it as a staccato 16th unless it is also
            # quiet, in which case it is a ghost to be dropped.
            if n.dur < grid * ghost_frac and n.vel < ghost_vel:
                dropped += 1
                continue
            gate = 1
        if step < 0:
            continue
        out.append((step, n.pitch, gate, n.vel, n.ch))
    out.sort(key=lambda e: (e[0], e[1]))
    return out, dropped


# --------------------------------------------------------------------------
# 3. Role classification
# --------------------------------------------------------------------------

DRUM_KICK = {35, 36}
DRUM_SNARE = {38, 40}
DRUM_HAT = {42, 44, 46}

# Stems that name their own role.
STEM_ROLE = {
    "bass": "bass",
    "brass": "stab",
    "fx": "fx",
}

def stem_name(path):
    base = os.path.basename(path)
    name = base[:-4] if base.lower().endswith(".mid") else base
    if "(" in name and name.endswith(")"):
        name = name[name.rindex("(") + 1 : -1]
    return name.strip().lower()


def drum_role(pitch):
    if pitch in DRUM_KICK:
        return "kick"
    if pitch in DRUM_SNARE:
        return "snare"
    if pitch in DRUM_HAT:
        return "hat"
    return "perc"


def stem_stats(events):
    """Register, polyphony and note length for one stem's pitched events."""
    if not events:
        return None
    pitch = sorted(e[1] for e in events)
    median = pitch[len(pitch) // 2]
    groups = {}
    for step, p, gate, vel, ch in events:
        groups.setdefault(step, []).append(p)
    poly = sum(len(v) for v in groups.values()) / len(groups)
    span = max(e[0] for e in events) - min(e[0] for e in events)
    dens = len(events) / max(span, 1)
    lengths = sorted(e[2] for e in events)
    return {
        "median": median,
        "poly": poly,
        "len": lengths[len(lengths) // 2],
        "dens": dens,
        "n": len(events),
    }


MELODIC_STEMS = {"guitar", "keyboard", "strings", "synth", "woodwinds"}
# A lead line is a written melody: it takes a real amount of material.
LEAD_MIN_NOTES = 40


def classify_song(stems):
    """Map stem -> role, by stem name first and statistics second.

    Named stems win outright: a `Bass` stem is the bass, a `Brass` stem is the
    brass stab, an `FX` stem is the one-shot FX layer.

    Everything else is pitched music:

    * the **lead** is the melodic stem whose writing is the most monophonic and
      the highest (a melody lives at the top of a range, one note at a time),
      among stems with enough material to be a line at all;
    * a stem that holds chords (`poly >= 2.2`, sustained) is a **pad**, and one
      that holds a stream of single notes is an **arpeggio**.

    Returns `(roles, stats)`.
    """
    roles = {}
    stats = {}
    pitched = {}
    for name, events in stems.items():
        rolled = [e for e in events if e[4] != 9]
        if not rolled:
            continue
        pitched[name] = rolled
        stats[name] = stem_stats(rolled)
        if name in STEM_ROLE:
            roles[name] = STEM_ROLE[name]

    def lead_score(name):
        s = stats[name]
        return s["median"] + 26.0 / (1.0 + s["poly"]) + 4.0 / (1.0 + s["len"])

    candidates = [
        name
        for name in pitched
        if name not in roles
        and name in MELODIC_STEMS
        and stats[name]["n"] >= LEAD_MIN_NOTES
    ]
    if candidates:
        roles[max(candidates, key=lead_score)] = "lead"
    for name, s in stats.items():
        if name in roles:
            continue
        if name not in MELODIC_STEMS:
            # Pitched leftovers on a drum stem are colour, not a part.
            continue
        roles[name] = "pad" if (s["poly"] >= 2.2 and s["len"] >= 2) else "arp"
    return roles, stats


def promote_top_line(parts, min_lead=10):
    """Give a song with no melody a lead voice: the top of the pad chords.

    A floor track needs something to carry the tune once the horde arrives, and
    a chordal stem already has one written into it -- its top voice. Moving that
    voice to the lead part (instead of doubling it) keeps the pad a clean chord
    and gives the sequencer a line. Used when no stem qualified as a lead, or
    when the stem that did has (almost) nothing inside the chosen loop.
    """
    if "pad" not in parts:
        return False
    if len(parts.get("lead", ())) >= min_lead:
        return False
    by_step = {}
    for step, pitch, gate, vel in parts["pad"]:
        by_step.setdefault(step, []).append((pitch, gate, vel))
    keep = []
    lead = []
    for step, notes in by_step.items():
        notes.sort(key=lambda n: -n[0])
        lead.append((step, notes[0][0], notes[0][1], notes[0][2]))
        for pitch, gate, vel in notes[1:]:
            keep.append((step, pitch, gate, vel))
    keep.sort(key=lambda n: (n[0], n[1]))
    lead.sort(key=lambda n: (n[0], n[1]))
    parts["pad"] = keep
    parts["lead"] = lead
    return True


def assign_roles(stems):
    """Every stem event tagged with a role.

    Channel 9 is General MIDI percussion whatever the stem is called, so those
    notes are classified by drum note first; anything else follows its stem.
    """
    roles, stats = classify_song(stems)
    tagged = []
    for name, events in stems.items():
        role = roles.get(name)
        for step, pitch, gate, vel, ch in events:
            if ch == 9:
                r = drum_role(pitch)
            elif role is None:
                continue
            else:
                r = role
            tagged.append((r, step, pitch, gate, vel))
    return tagged, roles, stats


# --------------------------------------------------------------------------
# 4. Clusters: merge, octave smear, caps
# --------------------------------------------------------------------------

def shape_clusters(tagged, steps_per_bar):
    """Merge duplicates and octave doublings, then cap polyphony per role.

    Per (role, step):

    * identical pitches collapse to the loudest;
    * an octave doubling (`p` and `p + 12`) keeps the upper note only -- that is
      the octave smear a wide stem writes, and one octave is plenty;
    * the highest `CAPS[role]` notes survive, except the bass, which keeps the
      lowest (the root), and the drums, where the GM note *is* the instrument so
      the cap counts distinct notes rather than loudness.
    """
    clusters = {}
    for role, step, pitch, gate, vel in tagged:
        clusters.setdefault((role, step), []).append((pitch, gate, vel))
    out = {}
    for (role, step), notes in clusters.items():
        notes.sort(key=lambda n: (-n[2], n[0]))
        kept = []
        seen = set()
        for pitch, gate, vel in notes:
            if pitch in seen:
                continue
            seen.add(pitch)
            kept.append((pitch, gate, vel))
        if role != "bass":
            present = {p for p, _, _ in kept}
            kept = [n for n in kept if not (n[0] + 12 in present)]
        if role == "bass":
            kept.sort(key=lambda n: n[0])
        else:
            kept.sort(key=lambda n: -n[0])
        cap = CAPS[role]
        if role in ("kick", "snare", "hat", "perc"):
            distinct = []
            for pitch, _gate, vel in kept:
                if all(p != pitch for p, _, _ in distinct):
                    distinct.append((pitch, 1, vel))
            kept = distinct[:cap]
        else:
            kept = kept[:cap]
        for pitch, gate, vel in kept:
            out.setdefault(role, []).append((step, pitch, max(gate, 1), vel))
    for role in out:
        out[role].sort(key=lambda n: (n[0], n[1]))
    return out


def legato(parts, steps):
    """Join a repeated pitch into one held note; trim overlaps.

    A stem that re-articulates the same pitch exactly where the previous note
    ended is one held note as far as a small sequencer is concerned, and a
    retrigger there is a click candidate for nothing. Monophonic roles (the
    bass) additionally cut the previous note at the new onset, so the bass part
    never needs a second voice.
    """
    mono = {"bass"}
    for role, notes in parts.items():
        out = []
        open_by_pitch = {}
        for step, pitch, gate, vel in notes:
            idx = open_by_pitch.get(pitch)
            if idx is not None:
                held = out[idx]
                end = held[0] + held[2]
                if step <= end:
                    held[2] = max(end, step + gate) - held[0]
                    held[3] = max(held[3], vel)
                    continue
            if role in mono and out:
                prev = out[-1]
                if prev[0] + prev[2] > step:
                    prev[2] = max(1, step - prev[0])
            out.append([step, pitch, max(gate, 1), vel])
            open_by_pitch[pitch] = len(out) - 1
        parts[role] = [(s, p, min(g, steps - s), v) for s, p, g, v in out if s < steps]
        parts[role].sort(key=lambda n: (n[0], n[1]))
    return parts


# Pitched roles in the order that wins a duplicated note. The stems in the
# supplied sets double each other a lot (a synth and a keyboard playing the
# same arpeggio, a guitar over the same chord), and playing both is just head-
# room spent on the same note: the higher-priority role keeps it.
DEDUPE_ORDER = ["lead", "stab", "pad", "arp", "fx"]


def dedupe_roles(parts):
    """Drop notes that a role with more claim to them already plays.

    Only exact `(step, pitch)` coincidences are removed, and the bass and the
    drums are never candidates (a bass doubling a pad root, or a kick under a
    tom, is arrangement, not a duplicate).
    """
    claims = set()
    out = {}
    for role in DEDUPE_ORDER:
        notes = parts.get(role)
        if not notes:
            continue
        kept = [n for n in notes if (n[0], n[1]) not in claims]
        claims.update((n[0], n[1]) for n in kept)
        if kept:
            out[role] = kept
    for role in ROLES:
        if role in out:
            continue
        if role in parts and (role not in DEDUPE_ORDER):
            out[role] = parts[role]
    return {role: out[role] for role in ROLES if role in out}


# --------------------------------------------------------------------------
# 5. Loop detection
# --------------------------------------------------------------------------

def bar_features(parts, steps_per_bar, bars):
    """One feature vector per bar: which grid steps each role sounds on.

    Rhythm and role activity are enough to find the form; pitch would make a
    transposed repeat look different.
    """
    width = len(ROLES) * steps_per_bar
    feats = []
    for bar in range(bars):
        v = [0.0] * width
        for role, notes in parts.items():
            r = R[role]
            for step, _pitch, gate, _vel in notes:
                if bar * steps_per_bar <= step < (bar + 1) * steps_per_bar:
                    pos = step - bar * steps_per_bar
                    for k in range(gate):
                        v[r * steps_per_bar + (pos + k) % steps_per_bar] = 1.0
        feats.append(v)
    return feats


def cosine(a, b):
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(y * y for y in b))
    if na < 1e-9 or nb < 1e-9:
        return 0.0
    return dot / (na * nb)


def find_loop(parts, steps_per_bar, bars, max_bars=32):
    """Find (start_bar, loop_bars, score, sections) -- one clean whole-bar loop.

    Two things decide the window:

    * **repetition** -- on the bar-feature vectors, how closely the bars after
      the window repeat the bars inside it. A loop that does not actually
      repeat would cut a phrase in half at the seam;
    * **coverage** -- how much of the song's material the window keeps. A loop
      that repeats beautifully but drops the lead for half of it is not the
      song. Coverage is measured per role, against that role's own density in
      the song, so a sparse FX layer counts as much as a wall of pads.

    Windows are whole bars and a multiple of four bars (four bars of 3/4 for
    the waltz), searched from the top of the song. The window with the best
    balance wins; ties go to the earlier, shorter window.
    """
    if bars <= 24:
        # A short arrangement (the sixteen-bar printworks piece) is its own
        # cycle: looping a quarter of it would throw away most of the song.
        loop_bars = max(8, (bars // 4) * 4)
        feats = bar_features(parts, steps_per_bar, bars)
        return 0, loop_bars, 0.0, detect_sections(feats, steps_per_bar, 0, loop_bars)
    feats = bar_features(parts, steps_per_bar, bars)
    totals = {role: len(notes) for role, notes in parts.items() if notes}
    candidates = []
    for loop_bars in range(8, min(bars, max_bars) + 1, 4):
        for start in range(0, bars - loop_bars + 1):
            score = 0.0
            pairs = 0
            for rep in range(start + loop_bars, bars - loop_bars + 1, loop_bars):
                for k in range(loop_bars):
                    score += cosine(feats[rep + k], feats[start + k])
                    pairs += 1
            sim = score / pairs if pairs else 0.0
            coverage = _coverage(parts, totals, start, loop_bars, steps_per_bar, bars)
            # Repetition is the point of a loop, so it leads; coverage breaks
            # the ties between windows that repeat about as well as each other.
            weight = 0.5 * sim + 0.5 * coverage
            candidates.append((weight, loop_bars, start, sim, coverage, pairs))
    if not candidates:
        return 0, min(bars, 16), 0.0, []
    weight, loop_bars, start, sim, _cov, _pairs = max(
        candidates, key=lambda c: (round(c[0], 3), -c[1], -c[2])
    )
    sections = detect_sections(feats, steps_per_bar, start, loop_bars)
    return start, loop_bars, sim, sections


def _coverage(parts, totals, start, loop_bars, steps_per_bar, bars):
    """How much of the song a window keeps.

    Half of it is *presence*: do the roles the song uses appear in the window
    at all (a loop that drops the lead is not the song). The other half is
    *density*: does the window carry those roles at their usual rate. Roles the
    song barely uses are ignored for presence, so one stray cymbal cannot veto
    an otherwise complete window.
    """
    lo = start * steps_per_bar
    hi = (start + loop_bars) * steps_per_bar
    used = {role: total for role, total in totals.items() if total >= 5}
    if not used or not totals:
        return 0.0
    present = 0
    for role in used:
        if any(lo <= n[0] < hi for n in parts[role]):
            present += 1
    presence = present / len(used)
    total_notes = sum(totals.values())
    expected = total_notes * (loop_bars / bars)
    got = sum(1 for role in totals for n in parts[role] if lo <= n[0] < hi)
    density = min(1.0, got / expected) if expected > 0 else 0.0
    return 0.6 * presence + 0.4 * density


def detect_sections(feats, steps_per_bar, start, loop_bars, min_len=4):
    """Split the loop into sections where the bar material changes.

    Two adjacent bars belong to the same section when their feature vectors are
    similar; a run shorter than `min_len` is folded into its neighbour so a
    one-bar fill is not called a section of its own. Only names/boundaries are
    reported (and emitted as data): the sequencer plays the whole loop as one
    cycle, so the form is preserved simply by not cutting it up.
    """
    bars = [feats[start + k] for k in range(loop_bars)]
    if len(bars) < 2 * min_len:
        return [(0, loop_bars)]
    cuts = [0]
    for k in range(1, len(bars)):
        if cosine(bars[k], bars[k - 1]) < 0.55:
            if k - cuts[-1] >= min_len:
                cuts.append(k)
    cuts.append(len(bars))
    sections = []
    for i in range(len(cuts) - 1):
        length = cuts[i + 1] - cuts[i]
        if sections and length < min_len:
            sections[-1] = (sections[-1][0], sections[-1][1] + length)
        else:
            sections.append((cuts[i], length))
    return sections


# --------------------------------------------------------------------------
# 6. Mix normalisation
# --------------------------------------------------------------------------

def note_amp(vel):
    """Note amplitude from MIDI velocity -- the synth's own mapping."""
    return 0.30 + 0.70 * (vel / 127.0)


def env_rms(role, gate_ms):
    """RMS of one voice's amplitude envelope over its whole life, in 1 ms steps.

    Mirrors the runtime envelope exactly: linear attack, then either a gated
    sustain (decayed only once the note's gate closes) or a one-pole decay that
    runs regardless, plus the release tail.
    """
    attack = ATTACK_MS[role]
    decay = DECAY_MS[role]
    release = RELEASE_MS
    gated = decay <= 0.0
    life = gate_ms + (release if gated else min(decay * 8.0, 4000.0))
    steps = max(4, int(life))
    acc = 0.0
    env = 0.0
    for ms in range(steps):
        if ms < attack:
            env = ms / attack
        elif gated and ms < gate_ms:
            env = 1.0
        elif gated:
            env *= math.exp(-1.0 / release)
        else:
            if ms >= attack:
                env = math.exp(-(ms - attack) / decay)
            if ms >= gate_ms:
                env *= math.exp(-1.0 / release)
        acc += env * env
    return math.sqrt(acc / steps)


def role_energy(parts, steps, step_ms):
    """Predicted RMS each role contributes to the mix, before its mix gain.

    For every grid step, the squared amplitudes of the voices sounding on that
    step add (different pitches are uncorrelated), and the role's waveform RMS
    scales the total. This is the same sum the runtime mix performs, so the
    per-role gains it produces land the mix where the table says.
    """
    energy = {}
    for role, notes in parts.items():
        power = [0.0] * steps
        for step, _p, gate, vel in notes:
            gate_ms = gate * step_ms
            a = note_amp(vel) * env_rms(role, gate_ms)
            live = max(1, int(round(gate))) if role not in ("kick", "snare", "hat", "perc") else max(1, int(DECAY_MS[role] / step_ms))
            for k in range(live):
                if step + k < steps:
                    power[step + k] += a * a
        mean_power = sum(power) / steps
        energy[role] = TIMBRE_RMS[role] * math.sqrt(mean_power)
    return energy


BUS_TRIM = 0.92  # mirrors `music::synth::BUS_TRIM`


def mix_gains(parts, steps, step_ms, drive, calm_rms=0.058, full_rms=0.15):
    """Per-role gains, level-matched to the background-music mix target.

    The calibration aims the boss tier at `full_rms` and the calm tier at no
    less than `calm_rms` (a sparse source -- the sixteen-bar printworks piece --
    has very little calm material, so its calm tier is the binding case). The
    trim that follows is the musical balance: bass and drums carry the floor,
    pads sit under the lead, FX only accents.
    """
    energy = role_energy(parts, steps, step_ms)
    # The saturator multiplies a quiet mix by `drive` before `BUS_TRIM`, so the
    # calibration is done on the level that actually leaves the bus.
    bus = drive * BUS_TRIM
    power = 0.0
    for role, e in energy.items():
        power += (e * LAYER_GAIN[role][3] * TIER_GAIN[3]) ** 2
    full = math.sqrt(power) * bus if power > 0 else 1.0
    power = 0.0
    for role, e in energy.items():
        power += (e * LAYER_GAIN[role][0] * TIER_GAIN[0]) ** 2
    calm = math.sqrt(power) * bus if power > 0 else 1.0
    scale = full_rms / full
    if calm * scale < calm_rms:
        # The calm tier is too thin to reach the band at this scale: lift the
        # whole song instead of leaving a silent menu track.
        scale = calm_rms / calm
    trim = {
        "kick": 1.05,
        "snare": 0.95,
        "hat": 0.80,
        "perc": 0.75,
        "bass": 1.20,
        "pad": 1.10,
        "arp": 0.85,
        "lead": 1.05,
        "stab": 0.90,
        "fx": 0.70,
    }
    gains = {role: round(scale * trim[role], 3) for role in ROLES}
    return gains, energy


def predict_rms(energy, gains, drive):
    """Predicted output RMS at each intensity tier (0..3)."""
    out = []
    for tier in range(4):
        power = 0.0
        for role in ROLES:
            if role not in energy:
                continue
            g = gains[role] * LAYER_GAIN[role][tier] * TIER_GAIN[tier]
            power += (energy[role] * g) ** 2
        out.append(round(math.sqrt(power) * drive * BUS_TRIM, 4))
    return out


# --------------------------------------------------------------------------
# 7. Song assembly
# --------------------------------------------------------------------------

class Song:
    def __init__(self):
        self.folder = ""
        self.module = ""
        self.theme = ""
        self.title = ""
        self.bpm = 120
        self.beats_per_bar = 4
        self.steps_per_bar = 16
        self.start_bar = 0
        self.bars = 16
        self.sections = []
        self.parts = {}
        self.gains = {}
        self.energy = {}
        self.roles = {}
        self.stats = {}
        self.dropped = 0
        self.promoted_lead = False
        self.raw_notes = 0
        self.loop_score = 0.0
        self.steps = 0
        self.step_ms = 120.0


def song_from_stems(folder, module, theme, title, midi_root, roll=False):
    paths = sorted(glob.glob(os.path.join(midi_root, folder, "*.mid")))
    if not paths:
        raise SystemExit(f"no .mid files under {os.path.join(midi_root, folder)}")
    stems = {}
    all_notes = []
    tempos = []
    division = None
    for path in paths:
        div, notes, tm = parse_midi(path)
        division = division or div
        if div != division:
            raise SystemExit(f"{path}: division {div} != {division}")
        all_notes.extend(notes)
        tempos.extend(tm)
        stems.setdefault(stem_name(path), []).extend(notes)
    if division is None or not all_notes:
        raise SystemExit(f"{folder}: no notes")

    song = Song()
    song.folder = folder
    song.module = module
    song.theme = theme
    song.title = title
    song.raw_notes = len(all_notes)
    song.bpm = resolve_bpm(tempos)
    song.beats_per_bar = detect_beats_per_bar(all_notes, division)
    song.steps_per_bar = 4 * song.beats_per_bar

    quantised = {}
    for name, notes in stems.items():
        events, dropped = quantise(
            notes, division, song.steps_per_bar, ghost_vel=26, ghost_frac=0.55
        )
        song.dropped += dropped
        if events:
            quantised[name] = events
    stems = quantised

    tagged, roles, stats = assign_roles(stems)
    song.roles = roles
    song.stats = stats
    parts = shape_clusters(tagged, song.steps_per_bar)
    total_steps = max(e[1] for e in tagged) + 2 * song.steps_per_bar
    bars = total_steps // song.steps_per_bar
    parts = legato(parts, bars * song.steps_per_bar)
    song.promoted_lead = promote_top_line(parts)

    start, loop_bars, score, sections = find_loop(parts, song.steps_per_bar, bars)
    song.start_bar = start
    song.bars = loop_bars
    song.loop_score = score
    song.sections = sections

    lo = start * song.steps_per_bar
    hi = (start + loop_bars) * song.steps_per_bar
    steps = hi - lo
    clipped = {}
    for role, notes in parts.items():
        kept = []
        for step, pitch, gate, vel in notes:
            if step < lo or step >= hi:
                continue
            gate = min(gate, hi - step)
            kept.append((step - lo, pitch, gate, vel))
        if kept:
            clipped[role] = kept
    if promote_top_line(clipped):
        song.promoted_lead = True
    song.parts = dedupe_roles(clipped)
    song.step_ms = 60_000.0 / (song.bpm * 4.0)
    song.gains, song.energy = mix_gains(
        clipped, steps, song.step_ms, STYLES[folder]["drive"]
    )
    song.steps = steps
    return song


# --------------------------------------------------------------------------
# 8. Rust emission
# --------------------------------------------------------------------------

def rust_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def encode_part(notes, steps):
    """Pack one role's notes into (delta, midi, gate, velocity) quads.

    Deltas are capped at 255 steps; a longer gap is written as a rest marker
    (`midi = 0`) so the table stays a flat u8 stream.
    """
    out = []
    cursor = 0
    for step, pitch, gate, vel in notes:
        while step - cursor > 255:
            out.append((255, 0, 0, 0))
            cursor += 255
        delta = step - cursor
        out.append((delta, pitch, min(gate, 255), max(1, min(127, vel))))
        cursor = step
    if not out:
        return out
    if sum(d for d, _p, _g, _v in out) + out[-1][2] > steps:
        raise SystemExit("part overruns the loop")
    return out


STYLE_KEYS = [
    "drive",
    "master",
    "bass_tri",
    "bass_lp",
    "lead_duty",
    "lead_lp",
    "pad_detune",
    "pad_lp",
    "arp_lp",
    "stab_duty",
    "stab_lp",
    "kick_hz",
]


def emit_rust(song, out_dir):
    steps = song.steps
    style = STYLES[song.folder]
    lines = []
    lines.append(f"//! {song.title} -- {song.theme} floor music.")
    lines.append("//!")
    lines.append("//! Generated, do not edit. Regenerate with:")
    lines.append("//!")
    lines.append(f"//!   python3 tools/midi2music.py <midi-root> src/music --only {song.folder}")
    lines.append("//!")
    lines.append(f"//! Song {song.folder}: {song.bpm} BPM, {song.beats_per_bar}/4, {song.bars}-bar loop")
    lines.append(f"//! starting at bar {song.start_bar} (loop match {song.loop_score:.2f}).")
    if song.promoted_lead:
        lines.append("//! No stem was a melody, so the pad's top voice became the lead part.")
    lines.append("//! Roles: " + ", ".join(
        f"{name}<-{stem}" for stem, name in sorted(song.roles.items())
    ))
    lines.append("//!")
    lines.append("//! A note is `(delta_steps, midi, gate_steps, velocity)`: the wait in 16th")
    lines.append("//! steps since the previous note of the same part, the MIDI (or General")
    lines.append("//! MIDI percussion) note, the gate in steps, and the MIDI velocity.")
    lines.append("")
    lines.append("use super::{Note, Song, Style};")
    lines.append("")
    lines.append(f"pub const BPM: u16 = {song.bpm};")
    lines.append(f"pub const BEATS_PER_BAR: u8 = {song.beats_per_bar};")
    lines.append(f"pub const BARS: u16 = {song.bars};")
    lines.append("")
    sec = ", ".join(f"({start}, {length})" for start, length in song.sections)
    lines.append(f"pub const SECTIONS: &[(u16, u16)] = &[{sec}];")
    lines.append("")
    for role in ROLES:
        notes = song.parts.get(role, [])
        packed = encode_part(notes, steps)
        lines.append(f"const {role.upper()}: &[Note] = &[")
        chunk = []
        for quad in packed:
            chunk.append("(%d,%d,%d,%d)" % quad)
            if len(chunk) == 8:
                lines.append("    " + ",".join(chunk) + ",")
                chunk = []
        if chunk:
            lines.append("    " + ",".join(chunk) + ",")
        lines.append("];")
        lines.append("")
    gains = ", ".join(f"{song.gains[r]:.3f}" for r in ROLES)
    lines.append("pub static STYLE: Style = Style {")
    lines.append(f"    mix: [{gains}],")
    for key in STYLE_KEYS:
        if key == "master":
            continue
        lines.append(f"    {key}: {style[key]:.2f},")
    lines.append("    ..Style::base()")
    lines.append("};")
    lines.append("")
    parts = ", ".join(r.upper() for r in ROLES)
    lines.append("pub static SONG: Song = Song {")
    lines.append(f"    name: {rust_str(song.title)},")
    lines.append(f"    floor: {rust_str(song.theme)},")
    lines.append("    bpm: BPM,")
    lines.append("    beats_per_bar: BEATS_PER_BAR,")
    lines.append("    bars: BARS,")
    lines.append("    sections: SECTIONS,")
    lines.append(f"    parts: [{parts}],")
    lines.append("    style: &STYLE,")
    lines.append("};")
    lines.append("")
    path = os.path.join(out_dir, song.module + ".rs")
    try:
        with open(path, "w") as fh:
            fh.write("\n".join(lines))
    except OSError as exc:
        raise SystemExit(f"midi2music: cannot write {path}: {exc}") from None
    return path, len("\n".join(lines))


def piano_roll(song, bars=8, width=None):
    """A text piano roll of the first `bars` bars of every role."""
    steps_per_bar = song.steps_per_bar
    limit = bars * steps_per_bar
    out = []
    for role in ROLES:
        notes = [n for n in song.parts.get(role, []) if n[0] < limit]
        if not notes:
            out.append(f"  {role:>5}: (silent)")
            continue
        lo = min(n[1] for n in notes)
        hi = max(n[1] for n in notes)
        if hi - lo > 24:
            hi = lo + 24
            notes = [n for n in notes if n[1] <= hi]
        used = sorted({n[1] for n in notes}, reverse=True)
        out.append(f"  {role:>5}: ({len(notes)} notes in {bars} bars, "
                   f"pitch {min(used)}..{max(used)})")
        for pitch in used:
            row = ["."] * limit
            for step, p, gate, _vel in notes:
                if p != pitch:
                    continue
                for k in range(gate):
                    if step + k < limit:
                        row[step + k] = "#" if k == 0 else "|"
            out.append(f"    {pitch:>3} {''.join(row)}")
        out.append("        " + "".join(
            "|" if (i % steps_per_bar) == 0 else " " for i in range(limit)
        ) + f"   ^ 1 bar = {steps_per_bar} steps ({song.beats_per_bar} beats)")
    return "\n".join(out)


def report(song, path, size):
    lines = []
    lines.append(f"== {song.module} ({song.title}, song {song.folder}) -> {path} ({size} B)")
    lines.append(
        f"   {song.bpm} BPM, {song.beats_per_bar}/4, loop bars {song.start_bar}..{song.start_bar + song.bars}"
        f" ({song.bars} bars, match {song.loop_score:.2f}), raw notes {song.raw_notes},"
        f" ghosts dropped {song.dropped}"
    )
    lines.append("   roles: " + ", ".join(
        f"{stem}->{role}" for stem, role in sorted(song.roles.items())
    ))
    for role in ROLES:
        notes = song.parts.get(role, [])
        if not notes:
            continue
        lines.append(
            f"     {role:>5}: {len(notes):5d} notes, gain {song.gains[role]:.3f},"
            f" energy {song.energy.get(role, 0.0):.3f}"
        )
    lines.append(
        "   predicted RMS by intensity 0..3: " + ", ".join(
            f"{x:.3f}"
            for x in predict_rms(song.energy, song.gains, STYLES[song.folder]["drive"])
        )
    )
    if song.promoted_lead:
        lines.append("   lead: extracted from the pad's top voice (no melodic stem)")
    lines.append("   sections (bar, bars): " + str(song.sections))
    return "\n".join(lines)


def main(argv):
    ap = argparse.ArgumentParser(description="MIDI stems -> Rust song tables")
    ap.add_argument("midi_root")
    ap.add_argument("out_dir")
    ap.add_argument("--only", default=None, help="only this song folder (1..6)")
    ap.add_argument("--roll", action="store_true", help="print a piano roll")
    ap.add_argument("--roll-bars", type=int, default=8)
    args = ap.parse_args(argv)

    try:
        os.makedirs(args.out_dir, exist_ok=True)
    except OSError as exc:
        raise SystemExit(f"midi2music: cannot create {args.out_dir}: {exc}") from None
    reports = []
    rolls = []
    for folder, module, theme, title in SONGS:
        if args.only and args.only != folder:
            continue
        song = song_from_stems(folder, module, theme, title, args.midi_root)
        path, size = emit_rust(song, args.out_dir)
        reports.append(report(song, path, size))
        if args.roll:
            rolls.append(f"--- {module} piano roll (first {args.roll_bars} bars) ---")
            rolls.append(piano_roll(song, args.roll_bars))
    print("\n".join(reports))
    if rolls:
        print("\n".join(rolls))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

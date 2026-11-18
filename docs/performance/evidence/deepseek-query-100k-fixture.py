import json, random, sys
random.seed(7)
N = 100000
credits = ["Hixxy & Sharkey","Hixxy / Dougal","Hixxy, Seduction","Earth, Wind & Fire","AC/DC","Unknown Artist No 7"]
genres = ["Jungle","Hardcore","Drum & Bass","Happy Hardcore","Techno","House"]
tracks = []
for i in range(N):
    artist = credits[i % len(credits)] if i % 11 else f"Artist {i}"
    tags = {
        "artist": artist,
        "albumartist": artist if i % 3 else f"Compilation {i % 40}",
        "album": f"Album {i}",
        "title": f"Track {i:06d}",
        "genre": genres[i % len(genres)],
        "year": str(1990 + i % 30),
        "track": str(1 + i % 20),
        "comment": f"note {i} 01:23:45",
        "lyrics": ("x" * 200) + f" body {i}",
        "custom:MOOD": ["Dark","Euphoric","Chill"][i % 3],
    }
    tracks.append({"path": f"/music/{i:06d}.flac", "format": "FLAC", "tags": tags,
                   "has_cover": i % 2 == 0})
def empty(text="", regex=False):
    return {"version":2,"text":text,"regex":regex,"caseSensitive":False,
            "root":{"kind":"group","id":"root","mode":"all","children":[]}}
def cond(field, op, values, cid="c1"):
    return {"version":2,"text":"","root":{"kind":"group","id":"root","mode":"all","children":[
        {"kind":"condition","id":cid,"field":field,"operator":op,"values":values}]}}
queries = [
  empty(),
  empty(text="jungle"),
  empty(text="genre:jungle"),
  empty(text="01:23:45:67:89"),
  cond("album","equals",["Album 12345"]),
  cond("genre","equals",["Jungle"]),
  cond("title","starts",["Track 0123"]),
  cond("artist","member",["Hixxy"]),
  cond("album","contains",["Album 1234"]),
]
out = {"tracks": tracks, "queries": queries, "iterations": 15,
       "suggest": {"text":"hi","field":None,"strategy":"values"}}
json.dump(out, sys.stdout)
print("synthetic rows:", N, file=sys.stderr)

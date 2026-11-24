// Writes the tree that the interop test of the Rust folder store compares with,
// and reads one the Rust store wrote. Both through the C# folder store.
//
//   dotnet run -- write DIR    puts the shared objects into DIR
//   dotnet run -- check DIR    lists and reads DIR and compares with the shared objects
using Stellarr.Api.Storage;

var objects = new (string Key, int Length, int Seed)[]
{
    ("images/2026/11/0f1e2d3c.png", 5000, 1),
    ("images/previews/2026/11/0f1e2d3c.webp", 300, 2),
    ("audio/2026/11/aa.webm", 70000, 3),
    ("videos/2026/11/bb.mp4", 1000000, 4),
    ("files/2026/11/cc.pdf", 0, 5),
    ("encrypted/2026/11/dd.bin", 10, 6),
    ("embeds/2026/11/ee.jpg", 10, 7),
    ("a/.dot/.hidden", 5, 8),
    ("żółć/ü.bin", 20, 9),
    ("with space/a b+c%d.bin", 7, 10),
};

static byte[] Bytes(int length, int seed)
{
    var bytes = new byte[length];
    for (var i = 0; i < length; i++)
    {
        bytes[i] = (byte)((i * 31) + (seed * 7) + (i >> 8));
    }

    return bytes;
}

var mode = args[0];
var storage = new FileSystemObjectStorage(args[1]);
var none = CancellationToken.None;

if (mode == "write")
{
    await storage.PrepareAsync(none);
    foreach (var (key, length, seed) in objects)
    {
        using var data = new MemoryStream(Bytes(length, seed));
        await storage.PutAsync(key, data, none);
    }

    Console.WriteLine($"wrote {objects.Length} objects");
    return 0;
}

var failures = 0;
var listed = new List<StoredObjectInfo>();
await foreach (var item in storage.ListAsync(none))
{
    listed.Add(item);
}

var expected = objects.OrderBy(o => o.Key, StringComparer.Ordinal).ToArray();
if (listed.Count != expected.Length)
{
    Console.WriteLine($"FAIL: listed {listed.Count} objects, expected {expected.Length}");
    failures++;
}

for (var i = 0; i < Math.Min(listed.Count, expected.Length); i++)
{
    if (listed[i].Key != expected[i].Key || listed[i].Size != expected[i].Length)
    {
        Console.WriteLine($"FAIL: listing {i} is {listed[i]}, expected {expected[i].Key} {expected[i].Length}");
        failures++;
    }
}

foreach (var (key, length, seed) in objects)
{
    var stored = await storage.GetAsync(key, null, none);
    await using var body = stored.Stream;
    using var copy = new MemoryStream();
    await body.CopyToAsync(copy);
    if (!copy.ToArray().AsSpan().SequenceEqual(Bytes(length, seed)))
    {
        Console.WriteLine($"FAIL: {key} reads back different");
        failures++;
    }
}

Console.WriteLine(failures == 0 ? $"OK: {objects.Length} objects listed and read" : $"{failures} failures");
return failures == 0 ? 0 : 1;

using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

// Ask an editing kit's own ManagedBlam whether it will load a tag.
//
// This is the only oracle that answers the question the user actually has --
// blam-tags re-reading its own output proves the file is self-consistent by
// blam-tags' rules, not that the mod tools accept it.
//
// Usage: probe.exe <kitRoot> <workList> <results>
//
// Work list lines are `<tag path without extension>|<extension>`. Results are
// tab separated -- a work line contains a `|` and a tag path cannot contain a
// tab, so this is the separator that can carry the line through as one key.
// Each is appended and flushed immediately, because a tag that takes
// the engine down takes this process with it -- the driver restarts from
// whatever the file already holds, so the crash is attributed to the one line
// that never got written.
static class Probe
{
    static string BinDir;
    static StreamWriter Results;
    static string Current = "";

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SetDllDirectory(string path);

    static int Main(string[] args)
    {
        if (args.Length < 3)
        {
            Console.Error.WriteLine("usage: probe.exe <kitRoot> <workList> <results>");
            return 2;
        }
        var kitRoot = args[0];
        BinDir = Path.Combine(kitRoot, "bin");

        // Both are needed before any Corinth/Bungie type is touched: the managed
        // half is found by the resolve handler, the native half by the DLL
        // search path.
        AppDomain.CurrentDomain.AssemblyResolve += (s, e) =>
        {
            var name = new AssemblyName(e.Name).Name;
            var path = Path.Combine(BinDir, name + ".dll");
            return File.Exists(path) ? Assembly.LoadFrom(path) : null;
        };
        SetDllDirectory(BinDir);

        var work = new List<string>();
        foreach (var line in File.ReadAllLines(args[1]))
            if (!string.IsNullOrWhiteSpace(line)) work.Add(line.Trim());

        // Anything already answered is left alone, which is what makes a restart
        // after a crash pick up rather than start over.
        var done = new HashSet<string>();
        if (File.Exists(args[2]))
            foreach (var line in File.ReadAllLines(args[2]))
            {
                var parts = line.Split('	');
                if (parts.Length >= 2) done.Add(parts[1]);
            }

        Results = new StreamWriter(args[2], append: true) { AutoFlush = true };
        try { return Run(kitRoot, work, done); }
        finally { Results.Dispose(); }
    }

    // Kept out of Main so the JIT does not resolve the engine types before the
    // resolve handler above is installed.
    [MethodImpl(MethodImplOptions.NoInlining)]
    static int Run(string kitRoot, List<string> work, HashSet<string> done)
    {
        Bungie.ManagedBlamSystem.Start(
            kitRoot,
            info => Report("CRASH", Current, Describe(info)),
            new Bungie.ManagedBlamStartupParameters());

        foreach (var line in work)
        {
            if (done.Contains(line)) continue;
            Current = line;
            var split = line.LastIndexOf('|');
            var relative = split < 0 ? line : line.Substring(0, split);
            var extension = split < 0 ? "" : line.Substring(split + 1);
            try
            {
                var path = Bungie.Tags.TagPath.FromPathAndExtension(relative, extension);
                using (var tag = new Bungie.Tags.TagFile(path))
                {
                    // Touching the root forces the whole structure to be walked,
                    // not just the header. A tag that opens and then falls over
                    // on its first field is not a tag that loaded.
                    var fields = tag.Fields;
                    Report("OK", line, fields == null ? "0 fields" : fields.Length + " fields");
                }
            }
            catch (Exception error)
            {
                var inner = error;
                while (inner.InnerException != null) inner = inner.InnerException;
                Report("FAIL", line, Flatten(inner.Message));
            }
        }
        Bungie.ManagedBlamSystem.Stop();
        return 0;
    }

    /// The crash info's shape is not documented here, so read whatever it has
    /// rather than assume a member name.
    static string Describe(object info)
    {
        if (info == null) return "engine crashed";
        var parts = new List<string>();
        foreach (var property in info.GetType().GetProperties())
        {
            object value = null;
            try { value = property.GetValue(info, null); } catch { }
            if (value != null) parts.Add(property.Name + "=" + Flatten(value.ToString()));
        }
        return parts.Count == 0 ? Flatten(info.ToString()) : string.Join(" ", parts);
    }

    static string Flatten(string text)
    {
        return (text ?? "").Replace("\r", " ").Replace("\n", " ").Replace("|", "/");
    }

    static void Report(string verdict, string line, string detail)
    {
        Results.WriteLine(verdict + "	" + line + "	" + detail);
        Results.Flush();
        Console.WriteLine(verdict + " " + line + " " + detail);
    }
}

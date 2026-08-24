using System;
using System.IO;
using System.Linq;
using System.Reflection;

// Dump the ManagedBlam surface we need to drive, so the probe is written
// against what the assembly actually declares rather than a guess.
static class Reflect
{
    static string BinDir;

    static int Main(string[] args)
    {
        BinDir = args[0];
        AppDomain.CurrentDomain.ReflectionOnlyAssemblyResolve += (s, e) =>
        {
            var name = new AssemblyName(e.Name).Name;
            var path = Path.Combine(BinDir, name + ".dll");
            if (File.Exists(path)) return Assembly.ReflectionOnlyLoadFrom(path);
            try { return Assembly.ReflectionOnlyLoad(e.Name); } catch { return null; }
        };

        var asm = Assembly.ReflectionOnlyLoadFrom(Path.Combine(BinDir, "ManagedBlam.dll"));
        Type[] types;
        try { types = asm.GetTypes(); }
        catch (ReflectionTypeLoadException ex) { types = ex.Types.Where(t => t != null).ToArray(); }

        var wanted = new[] { "ManagedBlamSystem", "ManagedBlamStartupParameters",
                             "ManagedBlamCrashCallback", "TagFile", "TagPath" };
        foreach (var type in types.Where(t => t != null && wanted.Contains(t.Name)).OrderBy(t => t.FullName))
        {
            Console.WriteLine("== " + type.FullName + (type.IsEnum ? " (enum)" : ""));
            foreach (var c in type.GetConstructors(BindingFlags.Public | BindingFlags.Instance))
                Console.WriteLine("   ctor(" + string.Join(", ", c.GetParameters().Select(p => p.ParameterType.Name + " " + p.Name)) + ")");
            foreach (var m in type.GetMethods(BindingFlags.Public | BindingFlags.Static | BindingFlags.Instance | BindingFlags.DeclaredOnly)
                                  .OrderBy(m => m.Name))
            {
                if (m.IsSpecialName && !m.Name.StartsWith("get_")) continue;
                Console.WriteLine("   " + (m.IsStatic ? "static " : "") + m.ReturnType.Name + " " + m.Name
                    + "(" + string.Join(", ", m.GetParameters().Select(p => p.ParameterType.Name + " " + p.Name)) + ")");
            }
            foreach (var f in type.GetFields(BindingFlags.Public | BindingFlags.Instance | BindingFlags.Static | BindingFlags.DeclaredOnly))
                Console.WriteLine("   field " + f.FieldType.Name + " " + f.Name);
        }
        return 0;
    }
}

// Offline updater fixture. Never loads Atsumi, WebView2, a profile or user data.
using System;
using System.IO;
using System.Threading;
public static class UpdateFixture {
    public static int Main(string[] args) {
        string root = AppDomain.CurrentDomain.BaseDirectory;
        if (args.Length == 1 && args[0] == "--wait") {
            File.WriteAllText(Path.Combine(root, "parent-ready"), "ready");
            for (int i = 0; i < 2400; i++) {
                if (File.Exists(Path.Combine(root, "exit-parent"))) return 0;
                Thread.Sleep(100);
            }
            return 2;
        }
        File.WriteAllText(Path.Combine(root, "restarted-fixture"), "synthetic update verified");
        return 0;
    }
}

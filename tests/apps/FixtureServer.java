import java.net.InetAddress;
import java.net.ServerSocket;

public class FixtureServer {
    public static void main(String[] args) throws Exception {
        try (ServerSocket server = new ServerSocket(0, 32, InetAddress.getLoopbackAddress())) {
            System.out.printf("{\"event\":\"ready\",\"pid\":%d,\"ports\":[%d],\"children\":[]}\n", ProcessHandle.current().pid(), server.getLocalPort());
            System.out.flush();
            Thread watchdog = new Thread(() -> {
                try { Thread.sleep(120000); } catch (InterruptedException ignored) {}
                System.exit(0);
            });
            watchdog.setDaemon(true);
            watchdog.start();
            while (System.in.read() != -1) {}
        }
    }
}

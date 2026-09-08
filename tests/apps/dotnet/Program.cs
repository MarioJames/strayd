using System;
using System.Net;
using System.Net.Sockets;
using System.Threading.Tasks;

using var listener = new TcpListener(IPAddress.Loopback, 0);
listener.Start();
Console.WriteLine($"{{\"event\":\"ready\",\"pid\":{Environment.ProcessId},\"ports\":[{((IPEndPoint)listener.LocalEndpoint).Port}],\"children\":[]}}");
_ = Task.Delay(120000).ContinueWith(_ => Environment.Exit(0));
while (Console.Read() != -1) {}

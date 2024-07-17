const WebSocket = require("ws");
const server = new WebSocket.Server({
  port: 8080,
});

let sockets = [];
server.on("connection", function (socket, req) {
  sockets.push(socket);

  console.log("Headers", req.headers);

  socket.on("message", function (msg) {
    console.log(JSON.parse(msg.toString("utf-8")));
  });

  socket.on("close", function () {
    sockets = sockets.filter((s) => s !== socket);
  });
});

import asyncio
import json
from urllib.request import Request, urlopen

import websockets


class FixtureRpc:
    def __init__(self, environment):
        self.environment = environment
        self.count = 0

    async def __aenter__(self):
        request = Request(
            self.environment["httpBaseUrl"] + "/api/auth/websocket-ticket",
            method="POST",
            headers={"Authorization": "Bearer " + self.environment["token"]},
        )

        def issue_ticket():
            with urlopen(request, timeout=10) as response:
                return json.load(response)["ticket"]

        ticket = await asyncio.to_thread(issue_ticket)
        self.socket = await websockets.connect(
            self.environment["httpBaseUrl"].replace("http", "ws", 1)
            + "/ws?wsTicket="
            + ticket
            + "&orchestrationProtocol=2",
            max_size=16 << 20,
        )
        return self

    async def __aexit__(self, *_):
        await self.socket.close()

    async def call(self, method, payload):
        self.count += 1
        request = str(self.count)
        await self.socket.send(
            json.dumps(
                {
                    "_tag": "Request",
                    "id": request,
                    "tag": method,
                    "headers": [],
                    "payload": payload,
                }
            )
        )
        async with asyncio.timeout(30):
            while True:
                reply = json.loads(await self.socket.recv())
                if (
                    reply.get("_tag") == "Exit"
                    and reply.get("requestId", reply.get("id")) == request
                ):
                    if reply["exit"]["_tag"] != "Success":
                        raise RuntimeError("Fixture RPC request failed: " + method)
                    return reply["exit"]["value"]

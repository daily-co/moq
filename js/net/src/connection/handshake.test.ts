import { expect, test } from "bun:test";
import { NativeSession } from "../ietf/adapter.ts";
import * as Message from "../ietf/message.ts";
import * as Namespace from "../ietf/namespace.ts";
import { Parameters, SetupOptions } from "../ietf/parameters.ts";
import { PublishNamespace } from "../ietf/publish_namespace.ts";
import { Publisher } from "../ietf/publisher.ts";
import { Setup } from "../ietf/setup.ts";
import { ALPN, Version } from "../ietf/version.ts";
import { createMockTransportPair } from "../mock.ts";
import { Producer as OriginProducer } from "../origin.ts";
import * as Path from "../path.ts";
import { Reader, Stream, Writer } from "../stream.ts";
import { exchangeSetup } from "./handshake.ts";

const HOP_ID = 0x40b54n;
const RELAY_COST = 0x40b56n;

test.each([Version.DRAFT_17, Version.DRAFT_18, Version.DRAFT_19, Version.DRAFT_20, Version.DRAFT_21, Version.DRAFT_22])(
	"cluster options leave modern version %i a plain client",
	async (version) => {
		const pair = createMockTransportPair(ALPN.DRAFT_19);
		const offered = new SetupOptions();
		offered.setVarint(HOP_ID, 42n);
		offered.setVarint(RELAY_COST, 3n);

		const peerWriter = await Writer.open(pair.client, { version });
		await peerWriter.u53(Setup.id);
		await new Setup({ parameters: offered }).encode(peerWriter, version);
		const handshake = await exchangeSetup(pair.server, version, "test");

		const incoming = pair.client.incomingUnidirectionalStreams.getReader();
		const next = await incoming.read();
		incoming.releaseLock();
		if (next.done) throw new Error("no SETUP response");
		const reader = new Reader(next.value, undefined, version);
		expect(await reader.u53()).toBe(Setup.id);
		const received = await Setup.decode(reader, version);
		expect(received.parameters.getVarint(HOP_ID)).toBeUndefined();
		expect(received.parameters.getVarint(RELAY_COST)).toBeUndefined();
		expect(handshake).not.toHaveProperty("cluster");

		const origin = new OriginProducer();
		const broadcast = origin.createBroadcast(Path.from("cam"));
		broadcast.announce();
		const publisher = new Publisher({
			quic: pair.server,
			session: new NativeSession(pair.server, version, false),
			publish: origin.consume(),
			requiresSolicitation: handshake.solicit ?? false,
		});
		const running = publisher.runPublishNamespaces();
		const stream = await Stream.accept(pair.client, version);
		if (!stream) throw new Error("no announcement");
		expect(await stream.reader.u53()).toBe(PublishNamespace.id);
		await Message.decode(stream.reader, async (body) => {
			await body.u62();
			if (version === Version.DRAFT_17) await body.u62();
			expect(await Namespace.decode(body)).toBe(Path.from("cam"));
			const parameters = await Parameters.decode(body, version);
			expect(parameters.vars.size).toBe(0);
			expect(parameters.bytes.size).toBe(0);
		});
		origin.close();
		pair.client.close();
		await running;
	},
);

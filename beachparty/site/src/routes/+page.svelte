<script lang="ts">
	import { onMount } from 'svelte';

	import Section from '../lib/section.svelte';
	import SignUp from '../lib/signup.svelte';

	import beach from '$lib/assets/beach.webp';
	import golf from '$lib/assets/golf.webp';
	import tennis from '$lib/assets/tennis.webp';
	import sailing from '$lib/assets/sailing.webp';
	import sunbathing from '$lib/assets/sunbathing.webp';
	import modernismo from '$lib/assets/modernismo.webp';
	import caleta from '$lib/assets/caleta.webp';
	import beachclub from '$lib/assets/beachclub_1.webp';
	import beachgym from '$lib/assets/beach_gym_1.webp';
	import croissant from '$lib/assets/croissant.webp';

	var events = [];
	var info = [];
	var lang = 'DE';

	onMount(async () => {
		// const recaptchaScript = document.createElement('script');
		// recaptchaScript.setAttribute('src', 'https://assets.calendly.com/assets/external/widget.js');
		// document.head.appendChild(recaptchaScript);
		const res = await fetch(`/api/events`);
		var data = await res.json();
		events = data['events'];
		console.log(events);
		info = data['info'];
		console.log(info);
	});
</script>

<div class="mx-8 my-4 flex flex-row space-x-4 text-xl">
	<label>
		<input type="radio" bind:group={lang} name="scoops" value={'DE'} class="apperance-none" />
		Deutsch
	</label>
	<label>
		<input type="radio" bind:group={lang} name="scoops" value={'ES'} />
		Español
	</label>
</div>

<Section background={beach} dir="left">
	<div slot="title" class="font-bold">
		BEACHPARTY <br />
		15-17 SEPT 2023 <br />
		SANT ANDREU DE LLAVANERES <br />
	</div>
	<div slot="content" class="flex flex-col align-start md:uppercase">
		<span class="font-bold my-4 text-amber-100"
			>{lang == 'DE' ? 'Freitag September 15' : 'Viernes Septiembre 15'}</span
		>
		<div class="grid grid-cols-2 grid-cols-[85px_auto] place-content-start gap-x-8">
			<span class="text-right text-amber-100">19:30</span>
			<span class="">
				{lang == 'DE' ? 'Drinks und Tapas in Can Markus' : 'Tapeo y copas en Can Markus'}
			</span>
		</div>
		<span class="font-bold my-4 text-amber-100"
			>{lang == 'DE' ? 'Samstag September 16' : 'Sábado Septiembre 16'}</span
		>
		<div class="grid grid-cols-2 grid-cols-[85px_auto] place-content-start gap-x-8">
			<span class="text-right text-amber-100">9:00 - 17:30</span>
			<span class="">
				{lang == 'DE'
					? 'Golf / Tennis / Segeln / Sonnen / Spaziergang / Altersgerechte gymnastik'
					: 'Golf / Tenis / Vela / Panching / Paseo modernista / Gimnasia playera'}
			</span>
			<span class="text-right text-amber-100">14:00</span>
			<span> Watering hole & Wine spot, La Caleta </span>
			<span class="text-right text-amber-100">15:00</span>
			<span>
				{lang == 'DE' ? 'Lange Siesta' : 'Siesta profunda'}
			</span>
			<span class="text-right text-amber-100">20:00</span>
			<span>Beachparty {lang == 'DE' ? 'im' : 'en'} GAS Quiet Club</span>
		</div>
		<span class="font-bold my-4 text-amber-100"
			>{lang == 'DE' ? 'Sonntag September 17' : 'Domingo Septiembre 17'}</span
		>
		<div class="grid grid-cols-2 grid-cols-[85px_auto] place-content-start gap-x-8">
			<span class="text-right text-amber-100">10:30</span>
			<span class="">Café & Croissant, Love</span>
		</div>
	</div>
</Section>

<Section background={golf}>
	<div slot="title">
		GOLF <br />
		9:30 / 9:40
	</div>
	<div slot="content">
		{lang == 'DE'
			? 'Leider nur für Frühaufsteher. Abschlag bei Tee 10.'
			: 'Solo para los madrugadores. Salida en el tee 10.'}
		<br />
		{lang == 'DE'
			? 'Dank noch vorhander Einladungen (60€) von Markus können wir die Greenfee (200€) wahrscheinlich senken.'
			: 'Gracias a las invitaciones aún disponibles (60€) de Markus se reduce probablemente el green fee (200€).'}
		<br />
		{lang == 'DE' ? 'Miete Golfschläger: 42€' : 'Alquiler de palos: 42€'}
		<br />

		Location:
		<a target="_blank" href="https://goo.gl/maps/oBozjG8pFH6khkGS7" class="hover:underline">
			Club de Golf de Llavaneres, Camí Golf, 49-51, Sant Andreu de Llavaneres
		</a>
	</div>
</Section>

<Section background={tennis} dir="left">
	<div slot="title">
		{lang == 'DE' ? 'Tennis / Padel' : 'Tenis / Padel'}
		<br />
		11:00 - 13:00
	</div>
	<div slot="content">
		{lang == 'DE' ? 'Einzel oder Doppel.' : 'Individual o dobles.'}
		<br />
		Location:
		<a target="_blank" href="https://goo.gl/maps/XWrdD2A6qd4DWhCR8" class="hover:underline">
			Tenis Mora, Camí de Can Pi, Sant Andreu de Llavaneres
		</a>

		<br />
	</div>
</Section>

<Section background={sailing} dir="right">
	<div slot="title">
		{lang == 'DE' ? 'Segeln' : 'Vela'}
		<br />
		11:00 - 13:00
	</div>
	<div slot="content">
		Location:
		<a target="_blank" href="https://goo.gl/maps/bNpxA4junTPRhK5y9" class="hover:underline">
			Club Náutico El Balís, Sant Andreu de Llavaneres.
		</a>
		{lang == 'DE' ? 'Liegeplatz' : 'Amarre'}: 468 - 470
	</div>
</Section>

<Section background={sunbathing} dir="left" gradient={false} blur={true}>
	<div slot="title" class="text-slate-800">
		{lang == 'DE' ? 'Sonnen' : 'Panching'}<br />
		12:00 - 18:00
	</div>
	<div slot="content" class="text-black">
		{lang == 'DE'
			? 'Vor Chiringuito Ohnades, bei schlechtem Wetter am Swimmingpool von Can Markus. '
			: 'Delante del chiringuito Ohnades, con mal tiempo en la piscina de Can Markus'}
		<br />
		Locations: <br />
		<a target="_blank" href="https://goo.gl/maps/74dJ5GVgj3zkKXnb8" class="hover:underline">
			Chiringuito Ohnades, Passeig del Marquès de Casa Riera, 25, 08394 Sant Vicenç de Montalt
		</a>
		<br />
		<a target="_blank" href="https://goo.gl/maps/hPoaACdjNvqbLUT4A" class="hover:underline">
			“Can Markus”, Avinguda Turo d’en Llull 64, 08392 Sant Andreu de Llavaneres
		</a>
	</div>
</Section>

<Section background={modernismo} dir="right">
	<div slot="title">
		{lang == 'DE' ? 'Spaziergang' : 'Paseo modernista'}
		<br />
		11:30
	</div>
	<div slot="content">
		{lang == 'DE' ? 'Spaziergang mit katalanischem Modernismus in' : 'En'}
		<a target="_blank" href="https://goo.gl/maps/USMcxD9bHwfiHV3b9" class="hover:underline">
			Canet de Mar.
		</a>
	</div>
</Section>

<Section background={caleta} dir="left">
	<div slot="title">
		Watering hole & Wine spot <br />
		14:00
	</div>
	<div slot="content">
		Location:
		<a target="_blank" href="https://goo.gl/maps/RDwLUgaAk2XUwCx5A" class="hover:underline">
			La Caleta, Passeig del Marquès de Casa Riera, 45, Sant Vicenç de Montalt
		</a>
		<br />
		<span class="font-bold my-2">
			{lang == 'DE' ? 'Mittagessen' : 'Comida: '} <br />
		</span>
		{lang == 'DE' ? 'Unsere Vorschläge:' : 'Nuestras propuestas:'}
		La Caleta, Ohnades, Mío, Sotavent, Tramendu Beach, Club de Golf.
		{lang == 'DE'
			? 'Sagt bitte Bescheid, wenn wir für Euch vorreservieren sollen.'
			: 'Por favor avisarnos si queréis que reservemos.'}
		<br />
	</div>
</Section>

<Section background={beachgym} dir="right" blur={true}>
	<div slot="title">
		{lang == 'DE' ? 'Altersgerechte Gymnastik' : 'Beach Gym'}
		<br />
		17:30
	</div>
	<div slot="content">
		Location:
		<a target="_blank" href="https://goo.gl/maps/RDwLUgaAk2XUwCx5A" class="hover:underline">
			Beach La Caleta, Passeig del Marquès de Casa Riera, 45, Sant Vicenç de Montalt
		</a>
	</div>
</Section>

<Section background={beachclub} dir="left" gradient={true}>
	<div slot="title">
		Beachparty <br />
		20:00
	</div>
	<div slot="content">
		{lang == 'DE' ? 'Im GAS Quiet Club.' : 'En el GAS Quiet Club.'}
		<br />
		{lang == 'DE'
			? 'Dresscode: Sommerlich mit Pullover und sandfesten Schuhen (am Abend kann es kühl werden)'
			: 'Dress code: Veraniego con jersey y zapatos a prueba de playa (por la noche refresca)'}
		<br />
		Location:
		<a target="_blank" href="https://goo.gl/maps/QVKbsvKvxKM4yViw9" class="hover:underline">
			GAS Quiet Club, Club Náutico El Balís, Sant Andreu de Llavaneres
		</a>
	</div>
</Section>

<Section background={croissant} dir="right" gradient={true}>
	<div slot="title">
		Café & Croissant <br />
		10:30
	</div>
	<div slot="content">
		{lang == 'DE' ? 'Am Strand.' : 'En la playa.'}
		<br />
		Location:
		<a target="_blank" href="https://goo.gl/maps/PgpTE5EGwk6htP7w6" class="hover:underline">
			Love Platja Xiringuito
		</a>
	</div>
</Section>

<div class="my-24 mx-6 md:mx-40 flex flex-col space-y-6">
	<h2 class="text-4xl font-display">
		{lang == 'DE' ? 'Anreise' : 'Llegada'}
	</h2>
	<div class="mx-6">
		{lang == 'DE'
			? 'Man kann vom Flughafen mit Bus und Bahn nach Caldes d‘ Estrac (Caldetas) kommen. Dauer: ca 2 Stunden'
			: 'Se puede llegar desde el aeropuerto a Caldes d’Estrac (Caldetes) con autobús y tren de cercanías. Duración: aprox 2 horas'}
		<br />
		<br />
		{lang == 'DE'
			? 'Von Terminal 1 oder 2 – Aerobus A1 nach Plaza Catalunya, dort in Rodalies, Linea R1,  Richtung Maçanet-Massanes.'
			: 'De la T1 o T2 con Aerobus A1 a Plaza Catalunya, desde ahí en Rodalies, Linea R1, dirección Maçanet-Massanes.'}
		<br />
		<br />
		{lang == 'DE' ? 'Per Taxi: 85€' : 'En Taxi: 85€'}
		<br />
		{lang == 'DE'
			? 'Taxi Cano, Miguel Angel: +34 606 679 953 (per whatsapp)'
			: 'Taxi Cano, Miguel Angel: +34 606 679 953 (per whatsapp)'}
		<br />
		{lang == 'DE'
			? 'Taxi Ferrer, Miguel, +34 670 408 444 (per whatsapp)'
			: 'Taxi Ferrer, Miguel, +34 670 408 444 (per whatsapp)'}
		<br />
		<br />
		{lang == 'DE'
			? 'Wenn Ihr möchtet, gebt Eure Flüge und Ankunftszeiten an. Wir helfen gerne, ob Ihr mit jemand anderen ein Taxi gemeinsam nehmen könnt.'
			: ''}
	</div>
</div>

<div class="my-24 mx-6 md:mx-40 flex flex-col space-y-6" id="partytraining">
	<h2 class="text-4xl font-display">
		{lang == 'DE' ? 'Semmelsches Partytraining' : 'Calentando motores a lo Semmel'}
	</h2>
	{#each events as event_type}
		<div class="mx-6">
			<span class="text-xl font-bold">{event_type.name}</span>
			<div class="grid grid-cols-2 grid-cols-[60px_auto] gap-x-4">
				{#each event_type.events as event}
					<span class="text-right"> {event['start']}</span>
					<div class="">
						{event['attendees'].join(', ')}
					</div>
				{/each}
			</div>
		</div>
	{/each}

	<SignUp {info} {events} />
</div>

<div class="w-1/1 h-[500px]">
	<iframe
		src="https://www.google.com/maps/d/u/0/embed?mid=1iIS9LaU-igfUyvDhNd9sEKQHznNirKo&ehbc=2E312F"
		width="100%"
		height="100%"
	/>
</div>

<div class="my-24 mx-6 md:mx-40 flex flex-col space-y-6">
	<h2 class="text-4xl font-display">
		{lang == 'DE' ? 'Adressen' : 'Direcciones'}
	</h2>
	<div class="mx-6">
		“Can Markus”, Avinguda Turo d’en Llull 64, 08392 Sant Andreu de Llavaneres <br />
		Club de Golf de Llavaneres, Camí Golf, 49-51, Sant Andreu de Llavaneres <br />
		Tenis Mora, Camí de Can Pi, Sant Andreu de Llavaneres <br />
		Club Náutico El Balís, Sant Andreu de Llavaneres, Amarres 468 – 470 <br />
		Chiringuito Ohnades, Passeig del Marquès de Casa Riera, 25, 08394 Sant Vicenç de Montalt <br />
		La Caleta, Passeig del Marquès de Casa Riera, 45, Sant Vicenç de Montalt, Tel. +34 937 911 558
		<br />
		Passeig del Marquès de Casa Riera, 41-42, 08394 Sant Vicenç de Montalt, +34 680 23 31 01 <br />
		Mío, Passeig del Marquès de Casa Riera, 1, Sant Vicenç de Montalt, +34 605 511 413 <br />
		Sotavent, Passeig dels Anglesos, 24, 08393 Caldes d'Estrac, +34 661 759 445 <br />
		GAS Quiet Club, Club Náutico El Balís, Sant Andreu de Llavaneres <br />
		Love, Passeig del Marquès de Casa Riera, 8-10, 08394 Sant Vicenç de Montalt <br />
	</div>
</div>

<div class="my-24 mx-6 md:mx-40 flex flex-col space-y-6">
	<h2 class="text-4xl font-display">
		{lang == 'DE' ? 'Telefone' : 'Móviles'}
	</h2>
	<div class="mx-6">
		Markus: +34 629 129 636 <br />
		Nata: +34 619 818 829 <br />
		Miriam: +34 638 089 540 <br />
		Frederik: +41 788 544 271 (WhatsApp) <br />
	</div>
</div>

<script lang="ts">
	import { onMount } from 'svelte';

	import sea from '$lib/assets/beach.webp';
	import croissant from '$lib/assets/croissant.webp';

	const midnight = Date.parse('2027-01-01T00:00:00+01:00');

	let left: number | null = null;

	onMount(() => {
		const tick = () => (left = Math.max(0, midnight - Date.now()));
		tick();
		const timer = setInterval(tick, 1000);
		return () => clearInterval(timer);
	});

	$: turned = left === 0;
	$: days = left === null ? 0 : Math.floor(left / 86_400_000);
	$: hours = left === null ? 0 : Math.floor(left / 3_600_000) % 24;
	$: minutes = left === null ? 0 : Math.floor(left / 60_000) % 60;
	$: seconds = left === null ? 0 : Math.floor(left / 1000) % 60;

	const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

	const days_ = [
		{
			date: 'Donnerstag, 31. Dezember',
			items: [
				{ time: '12:00', what: 'Vermut auf der Terrasse', where: 'Can Markus' },
				{ time: '14:00', what: 'Mittagessen am Meer', where: 'La Caleta' },
				{
					time: '16:30',
					what: 'Spaziergang am Strand bis Caldes d’Estrac, rechtzeitig zum Sonnenuntergang zurück',
					where: null
				},
				{ time: '21:00', what: 'Silvesterdinner zu siebt', where: 'Can Markus' },
				{
					time: '23:59',
					what: 'Zwölf Trauben, eine pro Glockenschlag. Wer alle schafft, hat ein gutes Jahr.',
					where: null
				},
				{ time: '0:00', what: 'Cava am Strand', where: 'Platja de Llavaneres' }
			]
		},
		{
			date: 'Freitag, 1. Januar',
			items: [
				{
					time: '12:00',
					what: 'Neujahrsbad im Meer, für alle, die sich trauen. Das Wasser hat um die 13 Grad.',
					where: null
				},
				{ time: '13:00', what: 'Café und Croissants', where: 'Can Markus' },
				{ time: 'danach', what: 'Lange Siesta', where: null }
			]
		}
	];

	const phones = [
		{ name: 'Markus', number: '+34 629 129 636' },
		{ name: 'Nata', number: '+34 619 818 829' },
		{ name: 'Miriam', number: '+34 638 089 540' },
		{ name: 'Frederik', number: '+41 78 854 42 71' }
	];
</script>

<svelte:head>
	<title>Silvester 2026 in Llavaneres</title>
</svelte:head>

<header class="hero" style="--sea: url({sea})">
	<div class="hero-inner">
		<p class="where">Silvester in Sant Andreu de Llavaneres</p>

		<h1 class="year" aria-label={turned ? '2027' : '2026'}>
			<span aria-hidden="true">202</span><span class="slot" aria-hidden="true"
				><span class="reel" class:turned><span>6</span><span>7</span></span></span
			>
		</h1>

		<p class="countdown" aria-live="off">
			{#if left === null}
				&nbsp;
			{:else if turned}
				Frohes neues Jahr.
			{:else if days > 0}
				Noch {plural(days, 'Tag', 'Tage')}, {plural(hours, 'Stunde', 'Stunden')} und
				{plural(minutes, 'Minute', 'Minuten')}.
			{:else}
				Noch <span class="clock"
					>{hours}:{String(minutes).padStart(2, '0')}:{String(seconds).padStart(2, '0')}</span
				>
			{/if}
		</p>

		<p class="who">Wir sieben in Can Markus, und um Mitternacht am Strand.</p>
	</div>
</header>

<main>
	<section class="programme" aria-labelledby="programm">
		<div>
			<h2 id="programm">Programm</h2>
			<p class="lede">Ein Vorschlag. Alles kann, nichts muss.</p>

			{#each days_ as day}
				<div class="day">
					<h3>{day.date}</h3>
					<ol>
						{#each day.items as item}
							<li>
								<span class="time">{item.time}</span>
								<span class="what">
									{item.what}
									{#if item.where}<span class="at">{item.where}</span>{/if}
								</span>
							</li>
						{/each}
					</ol>
				</div>
			{/each}
		</div>
		<img
			class="photo"
			src={croissant}
			alt="Croissants und Café zum Neujahrsfrühstück"
			loading="lazy"
		/>
	</section>

	<section class="info" aria-labelledby="wo">
		<div>
			<h2 id="wo">Wo</h2>
			<p>
				Can Markus<br />
				Avinguda Turó d’en Llull 64<br />
				08392 Sant Andreu de Llavaneres
			</p>
			<p>
				<a
					href="https://www.google.com/maps/search/?api=1&query=Avinguda+Tur%C3%B3+d%27en+Llull+64%2C+08392+Sant+Andreu+de+Llavaneres"
					>In Google Maps öffnen</a
				>
			</p>
			<p class="note">
				Vom Flughafen Barcelona eine knappe Stunde mit dem Auto, oder mit der R1 bis Sant Andreu de
				Llavaneres.
			</p>
		</div>

		<div>
			<h2 id="mitbringen">Mitbringen</h2>
			<ul class="bring">
				<li>Eine warme Jacke für Mitternacht am Strand</li>
				<li>Badesachen, falls ihr beim Neujahrsbad dabei seid</li>
				<li>Etwas Schönes zum Anziehen für das Dinner</li>
			</ul>
		</div>

		<div>
			<h2 id="telefon">Telefon</h2>
			<dl class="phones">
				{#each phones as p}
					<dt>{p.name}</dt>
					<dd><a href="tel:{p.number.replaceAll(' ', '')}">{p.number}</a></dd>
				{/each}
			</dl>
		</div>
	</section>
</main>

<footer>Bis bald am Meer.</footer>

<style>
	:global(html) {
		background: var(--night);
	}
	:global(body) {
		--night: #10284a;
		--deep: #0b1d37;
		--gold: #f3b33d;
		--cava: #f7ebc8;
		--glass: #7fc4b8;
		margin: 0;
		background: var(--night);
		color: var(--cava);
		font-family: 'Montserrat', system-ui, sans-serif;
		font-size: 1.125rem;
		line-height: 1.6;
	}
	:global(a) {
		color: var(--gold);
		text-underline-offset: 0.2em;
	}
	:global(a:focus-visible) {
		outline: 2px solid var(--gold);
		outline-offset: 3px;
	}

	.hero {
		position: relative;
		min-height: 100svh;
		display: flex;
		align-items: flex-end;
		background: linear-gradient(
				to bottom,
				var(--night) 0%,
				rgb(16 40 74 / 0.55) 45%,
				var(--night) 100%
			),
			var(--sea) center 70% / cover no-repeat;
		background-blend-mode: normal, luminosity;
		isolation: isolate;
	}
	.hero::before {
		content: '';
		position: absolute;
		inset: 0;
		background: var(--night);
		mix-blend-mode: color;
		z-index: -1;
	}
	.hero-inner {
		width: 100%;
		max-width: 72rem;
		margin: 0 auto;
		padding: 4rem 1.5rem 5rem;
	}
	.where {
		margin: 0;
		font-size: 1.125rem;
		color: var(--glass);
	}
	.year {
		margin: 0.25rem 0 0;
		font-family: 'Oswald', sans-serif;
		font-weight: 600;
		font-size: clamp(6rem, 44vw, 26rem);
		line-height: 1;
		letter-spacing: -0.01em;
		color: var(--gold);
		display: flex;
		align-items: flex-start;
	}
	.slot {
		display: block;
		height: 1em;
		overflow: hidden;
	}
	.reel {
		display: flex;
		flex-direction: column;
		transition: transform 1.6s cubic-bezier(0.7, 0, 0.2, 1);
	}
	.reel > span {
		display: block;
		height: 1em;
	}
	.reel.turned {
		transform: translateY(-1em);
	}
	.countdown {
		margin: 1.5rem 0 0;
		font-size: clamp(1.25rem, 3vw, 1.75rem);
		font-weight: 500;
		min-height: 1.6em;
	}
	.clock {
		font-variant-numeric: tabular-nums;
	}
	.who {
		margin: 0.5rem 0 0;
		max-width: 34rem;
		opacity: 0.8;
	}

	main {
		max-width: 72rem;
		margin: 0 auto;
		padding: 0 1.5rem;
	}
	h2 {
		font-family: 'Oswald', sans-serif;
		font-weight: 500;
		font-size: 2.5rem;
		line-height: 1.1;
		margin: 0;
		color: var(--gold);
	}
	.programme {
		padding: 6rem 0 2rem;
	}
	.lede {
		margin: 0.5rem 0 0;
		opacity: 0.8;
	}
	.day {
		margin-top: 3.5rem;
	}
	h3 {
		font-size: 1.25rem;
		font-weight: 600;
		margin: 0 0 1rem;
	}
	ol {
		list-style: none;
		margin: 0;
		padding: 0;
		border-left: 1px solid rgb(127 196 184 / 0.4);
	}
	li {
		display: grid;
		grid-template-columns: 5.5rem 1fr;
		gap: 1rem;
		padding: 0.75rem 0 0.75rem 1.25rem;
		position: relative;
		max-width: 46rem;
	}
	li::before {
		content: '';
		position: absolute;
		left: -4px;
		top: 1.35rem;
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--glass);
	}
	.time {
		color: var(--glass);
		font-variant-numeric: tabular-nums;
		font-weight: 600;
	}
	.at {
		display: block;
		font-size: 0.95rem;
		opacity: 0.7;
	}

	.photo {
		display: none;
	}
	@media (min-width: 64rem) {
		.programme {
			display: grid;
			grid-template-columns: 1fr 20rem;
			gap: 4rem;
			align-items: end;
		}
		.photo {
			display: block;
			width: 100%;
			aspect-ratio: 3 / 4;
			object-fit: cover;
		}
	}

	.info {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
		gap: 3rem;
		padding: 5rem 0 6rem;
	}
	.info p {
		margin: 1rem 0 0;
	}
	.note {
		font-size: 1rem;
		opacity: 0.8;
	}
	.bring {
		margin: 1rem 0 0;
		padding-left: 1.1rem;
	}
	.bring li {
		display: list-item;
		padding: 0 0 0.5rem;
	}
	.bring li::before {
		content: none;
	}
	.phones {
		margin: 1rem 0 0;
		display: grid;
		grid-template-columns: auto 1fr;
		gap: 0.25rem 1rem;
	}
	.phones dd {
		margin: 0;
		font-variant-numeric: tabular-nums;
	}

	footer {
		padding: 3rem 1.5rem 4rem;
		text-align: center;
		font-family: 'Oswald', sans-serif;
		font-size: 1.5rem;
		color: var(--gold);
		background: var(--deep);
	}

	@media (max-width: 40rem) {
		li {
			grid-template-columns: 4rem 1fr;
			gap: 0.75rem;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.reel {
			transition: none;
		}
	}
</style>

<script lang="ts">
	import { onMount } from 'svelte';

	import sea from '$lib/assets/beach.webp';
	import golf from '$lib/assets/golf.webp';
	import tennis from '$lib/assets/tennis.webp';
	import sailing from '$lib/assets/sailing.webp';
	import sunbathing from '$lib/assets/sunbathing.webp';
	import modernismo from '$lib/assets/modernismo.webp';
	import caleta from '$lib/assets/caleta.webp';
	import beachclub from '$lib/assets/beachclub_1.webp';
	import beachgym from '$lib/assets/beach_gym_1.webp';
	import croissant from '$lib/assets/croissant.webp';
	import Meme from '$lib/meme.svelte';

	const midnight = Date.parse('2027-01-01T00:00:00+01:00');

	let left: number | null = null;
	let meme: HTMLDialogElement;

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

	const maps = (query: string) =>
		`https://www.google.com/maps/search/?api=1&query=${encodeURIComponent(query)}`;

	const activities = [
		{ title: 'Golf', place: 'Club de Golf de Llavaneres', photo: golf },
		{ title: 'Tennis und Padel', place: 'Tenis Mora, Sant Andreu de Llavaneres', photo: tennis },
		{ title: 'Segeln', place: 'Club Nàutic El Balís', photo: sailing },
		{ title: 'Am Strand liegen', place: 'Ohnades, Sant Vicenç de Montalt', photo: sunbathing },
		{ title: 'Modernisme in Canet de Mar', place: 'Canet de Mar', photo: modernismo },
		{ title: 'Wein am Meer', place: 'La Caleta, Sant Vicenç de Montalt', photo: caleta },
		{
			title: 'Strandgymnastik',
			place: 'Platja de la Caleta, Sant Vicenç de Montalt',
			photo: beachgym
		},
		{ title: 'Drinks im Club', place: 'GAS Quiet Club, Port del Balís', photo: beachclub },
		{
			title: 'Café und Croissant',
			place: 'Love Platja Xiringuito, Sant Vicenç de Montalt',
			photo: croissant
		}
	];
</script>

<svelte:head>
	<title>Silvester 2026 in Llavaneres</title>
</svelte:head>

<header class="hero" style="--sea: url({sea})">
	<div class="hero-inner">
		<p class="where">Silvester zu siebt in Sant Andreu de Llavaneres</p>

		<h1 class="year" aria-label={turned ? '2027' : '2026'}>
			<span aria-hidden="true">202</span><span class="slot" aria-hidden="true"
				><span class="reel" class:turned><span>6</span><span>7</span></span></span
			>
		</h1>

		<p class="countdown">
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

		<button class="in" type="button" on:click={() => meme.showModal()}>
			Son of a bitch, I’m in
		</button>
	</div>
</header>

<main>
	<h2>Was wir machen können</h2>

	{#each activities as a, i}
		<section class="activity" class:right={i % 2 === 1} style="--photo: url({a.photo})">
			<div class="activity-inner">
				<h3>{a.title}</h3>
				<a href={maps(a.place)} target="_blank" rel="noreferrer">{a.place}</a>
			</div>
		</section>
	{/each}
</main>

<footer>Bis bald am Meer.</footer>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
	bind:this={meme}
	on:click={(e) => {
		if (e.target === meme) meme.close();
	}}
	aria-label="Son of a bitch, I’m in"
>
	<Meme />
	<form method="dialog">
		<button class="close" type="submit">Schliessen</button>
	</form>
</dialog>

<style>
	:global(html) {
		background: #10284a;
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
	:global(a:focus-visible),
	:global(button:focus-visible) {
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
	.in {
		margin-top: 2rem;
		padding: 0.9rem 1.6rem;
		border: none;
		border-radius: 999px;
		background: var(--gold);
		color: var(--night);
		font: inherit;
		font-weight: 700;
		font-size: 1.125rem;
		cursor: pointer;
		transition: transform 0.15s ease;
	}
	.in:hover {
		transform: rotate(-2deg) scale(1.04);
	}
	.in:active {
		transform: scale(0.97);
	}

	main > h2 {
		max-width: 72rem;
		margin: 0 auto;
		padding: 5rem 1.5rem 2.5rem;
		font-family: 'Oswald', sans-serif;
		font-weight: 500;
		font-size: 2.5rem;
		line-height: 1.1;
		color: var(--gold);
	}

	.activity {
		min-height: clamp(18rem, 55vh, 32rem);
		display: flex;
		align-items: flex-end;
		background: linear-gradient(
				to top,
				rgb(16 40 74 / 0.95) 0%,
				rgb(16 40 74 / 0.2) 55%,
				rgb(16 40 74 / 0) 100%
			),
			var(--photo) center / cover no-repeat;
	}
	.activity-inner {
		width: 100%;
		max-width: 72rem;
		margin: 0 auto;
		padding: 2.5rem 1.5rem;
		display: flex;
		flex-direction: column;
		align-items: flex-start;
	}
	.activity.right .activity-inner {
		align-items: flex-end;
		text-align: right;
	}
	h3 {
		margin: 0;
		font-family: 'Oswald', sans-serif;
		font-weight: 500;
		font-size: clamp(2rem, 6vw, 3.75rem);
		line-height: 1.1;
		letter-spacing: 0.2em;
		text-transform: uppercase;
		color: #fff;
	}
	.activity a {
		margin-top: 0.5rem;
		color: var(--gold);
		text-underline-offset: 0.2em;
	}

	footer {
		padding: 4rem 1.5rem;
		text-align: center;
		font-family: 'Oswald', sans-serif;
		font-size: 1.5rem;
		color: var(--gold);
		background: var(--deep);
	}

	dialog {
		padding: 0;
		border: none;
		background: transparent;
		max-width: min(36rem, 92vw);
		width: 100%;
	}
	dialog::backdrop {
		background: rgb(11 29 55 / 0.85);
	}
	dialog[open] {
		animation: pop 0.35s cubic-bezier(0.2, 1.4, 0.4, 1);
	}
	.close {
		display: block;
		margin: 1rem auto 0;
		padding: 0.5rem 1.25rem;
		border: 1px solid var(--cava);
		border-radius: 999px;
		background: transparent;
		color: var(--cava);
		font: inherit;
		cursor: pointer;
	}
	@keyframes pop {
		from {
			transform: scale(0.6) rotate(-6deg);
			opacity: 0;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.reel,
		.in {
			transition: none;
		}
		dialog[open] {
			animation: none;
		}
	}
</style>

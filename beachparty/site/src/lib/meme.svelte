<script lang="ts">
	const supplied = Object.values(
		import.meta.glob('./meme/*.{jpg,jpeg,png,webp}', { eager: true, import: 'default' })
	) as string[];
	const picture = supplied[0];
</script>

<figure class="meme">
	{#if picture}
		<img src={picture} alt="" />
	{:else}
		<svg viewBox="0 0 600 600" role="img" aria-label="Eine Semmel, die dabei ist">
			<defs>
				<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
					<stop offset="0" stop-color="#0b1d37" />
					<stop offset="1" stop-color="#1e4a7a" />
				</linearGradient>
				<radialGradient id="crust" cx="0.4" cy="0.35" r="0.75">
					<stop offset="0" stop-color="#f6c66a" />
					<stop offset="0.6" stop-color="#d9913a" />
					<stop offset="1" stop-color="#a8641f" />
				</radialGradient>
			</defs>
			<rect width="600" height="600" fill="url(#sky)" />
			<g fill="#f7ebc8">
				<circle cx="90" cy="110" r="2.5" />
				<circle cx="510" cy="80" r="2" />
				<circle cx="430" cy="150" r="1.5" />
				<circle cx="160" cy="60" r="1.5" />
				<circle cx="540" cy="220" r="2" />
			</g>
			<rect y="440" width="600" height="160" fill="#10284a" />
			<path d="M0 440 Q150 425 300 440 T600 440" stroke="#7fc4b8" stroke-width="3" fill="none" />

			<g transform="translate(300 330)">
				<ellipse cx="0" cy="95" rx="190" ry="24" fill="#0b1d37" opacity="0.6" />
				<ellipse cx="0" cy="0" rx="190" ry="150" fill="url(#crust)" />
				<g stroke="#8a4f17" stroke-width="7" stroke-linecap="round" fill="none" opacity="0.75">
					<path d="M0 -150 Q-10 -110 -5 -78" />
					<path d="M-150 -80 Q-110 -80 -85 -60" />
					<path d="M150 -80 Q110 -80 85 -60" />
				</g>

				<path
					d="M-95 -38 Q-62 -62 -30 -44"
					stroke="#5a3210"
					stroke-width="10"
					stroke-linecap="round"
					fill="none"
				/>
				<path
					d="M32 -56 Q66 -50 96 -30"
					stroke="#5a3210"
					stroke-width="10"
					stroke-linecap="round"
					fill="none"
				/>
				<ellipse cx="-62" cy="-5" rx="30" ry="36" fill="#fff" />
				<ellipse cx="62" cy="-5" rx="30" ry="32" fill="#fff" />
				<circle cx="-52" cy="0" r="13" fill="#10284a" />
				<circle cx="72" cy="0" r="13" fill="#10284a" />
				<circle cx="-48" cy="-5" r="4" fill="#fff" />
				<circle cx="76" cy="-5" r="4" fill="#fff" />

				<path
					d="M-70 62 Q0 118 80 50"
					stroke="#5a3210"
					stroke-width="9"
					stroke-linecap="round"
					fill="#7a2f1a"
				/>
				<path d="M-58 66 Q0 104 68 58 L64 66 Q0 94 -52 72 Z" fill="#fff" />
			</g>
		</svg>
	{/if}
	<figcaption>Son of a bitch,<br />I’m in</figcaption>
</figure>

<style>
	@font-face {
		font-family: 'Anton';
		src: url('/fonts/Anton-Regular.ttf') format('truetype');
		font-display: block;
	}
	.meme {
		position: relative;
		margin: 0;
		aspect-ratio: 1;
		overflow: hidden;
		border-radius: 0.5rem;
		box-shadow: 0 1.5rem 4rem rgb(0 0 0 / 0.5);
	}
	img,
	svg {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: cover;
	}
	figcaption {
		position: absolute;
		left: 0;
		right: 0;
		bottom: 4%;
		padding: 0 4%;
		text-align: center;
		font-family: 'Anton', Impact, sans-serif;
		font-size: clamp(2rem, 9vw, 3.4rem);
		line-height: 1.05;
		text-transform: uppercase;
		color: #fff;
		-webkit-text-stroke: 0.06em #000;
		paint-order: stroke fill;
		text-shadow: 0 0.04em 0.08em rgb(0 0 0 / 0.6);
	}
</style>

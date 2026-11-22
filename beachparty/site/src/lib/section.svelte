<script lang="ts">
	export let background: string;
	export let gradient: boolean = true;
	export let blur: boolean = false;
	export let dir = 'right';

	function dir_to_flex(dir: string): string {
		if (dir == "right") {
			return "flex-end"
		}
		return "flex-start"
	}
</script>

<div class="relative overflow-hidden text-white">
	<div class="relative z-10 flex flex-col overflow-hidden space-y-16 md:space-y-32 my-24 mx-6 md:mx-40" 
			 style="align-items: {dir_to_flex(dir)}">
		<div
			class="flex flex-col text-4xl md:text-6xl font-display tracking-[0.2em] uppercase items-end"
			style="text-align: {dir}"
		>
			<slot name="title"/>
		</div>
		<div class="flex flex-col text-xl md:text-2xl font-body">
			<slot name="content" />
		</div>
	</div>
	<div
		class="absolute bottom-0 w-full h-full bg-cover bg-bottom"
		style="background-image: url({background});"
	/>
	{#if gradient}
		<div
			class="absolute bottom-0 w-full h-full z-5"
			style="background: linear-gradient(to {dir}, rgba(0,0,0,0) 0%, rgba(0,0,0,0.40) 100%);"
		/>
	{/if}
	{#if blur}
		<div
			class="absolute bottom-0 w-full h-full z-5 backdrop-blur-sm"
		/>
	{/if}
</div>

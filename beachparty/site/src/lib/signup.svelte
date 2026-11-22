<script lang="ts">
	// let valid_activities = ['Tennis', 'Padel', 'Golf', 'Sailing 4Y'];
	// let activity = valid_activities[0];

	export let info: EventInfo[] = [];
	export let events: Event[] = [];

	let activity: string;
	let start: string;
	let name: string;

	type EventInfo = {
		name: string;
		description: string;
		location: string;
		duration: string;
		max_people_per_slot: number;
		slots: string[];
	};

	type Event = {
		name: string;
		events: {
			start: string;
			attendees: string[];
		}[];
	};

	type SlotInfo = {
		slot: string;
		current_participants: number;
		max_participants: number;
	};

	function valid_activities(info: EventInfo[]): string[] {
		let activityNames: string[] = [];

		for (let activity of info) {
			activityNames.push(activity.name);
		}
		return activityNames;
	}

	function available_slots_for(
		activityName: string,
		activities: EventInfo[],
		events: Event[]
	): SlotInfo[] {
		let activity = activities.find((a) => a.name === activityName);
		let event = events.find((e) => e.name === activityName);

		if (!activity) {
			return [];
		}

		if (!event) {
			console.log('No event');
			return activity.slots.map((slot) => ({
				slot,
				current_participants: 0,
				max_participants: activity.max_people_per_slot
			}));
		}
		// Convert start times to only hour and minute, to match the format in slots
		// let occupiedSlots = event.events.map((e) => e.start);

		// Count the number of events happening at each slot
		let slotCounts: { [key: string]: number } = {};
		event.events.forEach((slot) => {
			slotCounts[slot.start] = (slotCounts[slot.start] || 0) + slot.attendees.length;
		});
		console.log(slotCounts);

		// Build an array of SlotInfo objects for slots where the count is less than the max people per slot
		let availableSlots: SlotInfo[] = activity.slots
			.filter((slot) => !slotCounts[slot] || slotCounts[slot] < activity.max_people_per_slot)
			.map((slot) => ({
				slot,
				current_participants: slotCounts[slot] || 0,
				max_participants: activity.max_people_per_slot
			}));

		return availableSlots;
	}
	// async function handleSubmit() {
	// 	console.log(activity);
	// 	console.log(name);
	// 	console.log(start);
	//    const params = new URLSearchParams();
	//    params.append('name', name);
	//    params.append('activity', activity);
	//    params.append('start', start);

	//    const response = await fetch('/api/register', {
	//        method: 'POST',
	//        headers: {
	//            'Content-Type': 'application/x-www-form-urlencoded'
	//        },
	//        body: params
	//    });
	// 	if (response.ok) {
	// 		// const result = await response.json();
	// 		// Handle success - show message to user, clear form, etc.
	// 	} else {
	// 		// Handle error - show error message, etc.
	// 	}
	// }
</script>

<div class="mx-6">
	<span class="text-xl font-bold">Anmeldung</span>
	<form method="POST" class="w-full max-w-lg mx-auto" action="/api/register">
		<div class="grid grid-cols-1 sm:grid-cols-2 gap-1 mb-4">
			{#each info as act}
				<label class="inline-flex items-center">
					<input
						name="activity"
						type="radio"
						bind:group={activity}
						value={act.name}
						class="text-blue-600"
					/>
					<span class="ml-2">{act.name}</span>
				</label>
			{/each}
		</div>
		<label class="block">
			<span class="text-gray-700">Name</span>
			<input
				name="name"
				bind:value={name}
				type="text"
				class="mt-1 block w-full rounded-md border-gray-300 border-2 shadow-sm"
			/>
		</label>
		<div class="mt-4">
			<span class="">Start time</span>
			{#each available_slots_for(activity, info, events) as slot_info}
				<label class="block mt-2">
					<input
						name="start"
						type="radio"
						bind:group={start}
						value={slot_info.slot}
						class="text-blue-600"
					/>
					<span class="ml-2"
						>{slot_info.slot} ({slot_info.current_participants} / {slot_info.max_participants})</span
					>
				</label>
			{/each}
		</div>
		<button class="mt-6 bg-blue-500 hover:bg-blue-700 text-white font-bold py-2 px-4 rounded"
			>Submit</button
		>
	</form>
</div>

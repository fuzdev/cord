<script lang="ts">
	import { resolve } from '$app/paths';
</script>

<svelte:head>
	<title>admin - Cord</title>
</svelte:head>

<div class="p_lg">
	<h1>admin</h1>
	<p>Manage who can sign up and who's signed in.</p>
	<nav>
		<ul>
			<li><a href={resolve('/admin/invites')}>invites</a>: signup invites</li>
			<li><a href={resolve('/admin/accounts')}>accounts</a>: accounts and role grants</li>
			<li><a href={resolve('/admin/sessions')}>sessions</a>: active sessions</li>
			<li><a href={resolve('/admin/audit')}>audit log</a>: security events</li>
			<li><a href={resolve('/account')}>your account</a>: your own sessions</li>
		</ul>
	</nav>
</div>

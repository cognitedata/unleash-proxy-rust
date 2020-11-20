# A web browser proxy for unleash

This accepts an unleash context as GET query parameters, evaluates all the
toggles present in the Unleash API and returns a JSON document with their state.
Custom properties can be submitted by passing `properties[name]=value` in the
query string. Percent decoding is handled: a percent encoded name, or value,
will be passed to the Unleash API client in decoded form.

It also accepts Metrics documents from clients and aggregates them before
passing onto the upstream Unleash API (protecting the API in case of DOS attack
or similar).

The contract that this proxy implements is not precisely specified; instead it
matches the API expected by the JS client at
https://github.com/unleash-hosted/unleash-proxy-client-js/; if that JS client
does not work with this proxy, it is a bug.
    
The current code base does not yet implement variants (as the rust client does
not yet do that), nor client authentication (but that should perhaps be done in
an ingress layer rather than in the proxy itself).

Performance wise, this shows acceptable performance today - approximately 0.05ms
per request.

## Custom Strategies

As Unleash implements custom stratgies client-side, custom strategies will
require a custom proxy; for now there is no plugin-approach implemented: make a
branch or other fork of the code and implement custom strategies in that branch;
we won't merge strategies that are not relevant to the upstream Unleash API into
the master branch of the proxy.

## Copyright

This code is copyright (c) 2020 Cognite AS, all rights reserved.

It is not yet licensed for redistribution.
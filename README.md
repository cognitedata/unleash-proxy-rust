# A web browser proxy for unleash

[Unleash](https://unleash.github.io) is a feature flag API system. This is a web
service that accepts an unleash context as GET query parameters, evaluates all
the toggles present in the Unleash API and returns a JSON document with their
state. Custom properties can be submitted by passing `properties[name]=value` in
the query string. Percent decoding is handled: a percent encoded name, or value,
will be passed to the Unleash API client in decoded form.

It also accepts Metrics documents from clients and aggregates them before
passing onto the upstream Unleash API (protecting the API in case of DOS attack
or similar).

The contract that this proxy implements is not precisely specified; instead it
matches the API expected by the JS client at
https://github.com/unleash-hosted/unleash-proxy-client-js/; if that JS client
does not work with this proxy, it is a bug.
    
The current code base does not yet implement variants (though as the rust client
does this should be straight forward now), nor client authentication (but that
should perhaps be done in an ingress layer rather than in the proxy itself).

Performance wise, this shows acceptable performance today - approximately 0.05ms
per request.

## Custom Strategies

As Unleash implements custom strategies client-side, custom strategies will
require a custom proxy; see the ProxyBuilder struct in the crate to permit low-touch implementations.

## Copyright

This code is copyright (c) 2020-2022 Cognite AS, all rights reserved.

It is licensed under the Apache V2 License.


## status

While it doesn't support everything Unleash does, it should be very stable with the subset it does support. We are not actively making it better today, but neither it is forgotten.

## Code of conduct

Please note that this project is released with a Contributor Code of Conduct. By
participating in this project you agree to abide by its terms.

## Contributing

PR's on Github as normal please. Cargo test to run the test suite, rustfmt code
before submitting. Testing is entirely manual today; that would be great to change.

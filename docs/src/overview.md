# Overview

Throughout the rest of the _User Documentation_ you'll see us talking about RTF both as the
framework itself and as part of the richer tooling and testing capabilities provided by the Runtime
Readiness team. We feel that it is important to draw a distinction between these two areas in order
to be able to make the best use of each.

So, while the rest of our documentation tends to focus on the "how" of using RTF and its related
tooling, this page instead covers some of the "why" around how things are set up and the different
options available for making use of it. To kick things off, its probably best to cover what RTF _is_
(and importantly, what it _isn't_).

## What RTF _is_

### Compositional Glue

The most important thing to understand about RTF is that it truly is _all_ glue code and meta-data.
RTF Test plans follow a simple "setup, run, teardown" execution model and providers are all self
contained functions of their inputs. Need access to a resource inside of your test scenario? Add the
appropriate provider. Need access to the same resource when you're spinning up your test
environment? Add the provider there as well: RTF will handle de-duplicating the resources for you.

### Once size fits most (not all)

Aiming to provide a working out of the box solution for every use case we come across inevitably
results in chasing a long tail infrequently used aspects of the system that bitrot or are
insufficiently tested.

No one wants that.

Instead we focus on paving a path for the majority of use cases that we know are actively required
by our users (initially the Router core team) while ensuring that the system remains open to
extension. As and when new shared use cases arise we work with the users who have an interest in
their semantics to pull them in to the managed set of providers and base configurations.

## What RTF is _not_

### A magic bullet

While it is certainly possible to rewrite all of your existing test suites to run under RTF we
wouldn't recommend it. For anyone. The value add for using a framework like RTF is that it can
handle pulling together resources and test data for you in a standardised, reliable way. If all you
need to do is run something that looks like a unit test with no external dependencies, then RTF is
almost guaranteed to be overkill.

### Purpose built for your exact use case

By design, RTF is a general purpose framework that focuses on giving users the tools to write their
own test plans and tooling. Given the nature of the types of tests we typically see run under RTF,
you may be inclined to ask why the framework doesn't offer built in support for things like running
Terraform or managing Kubernetes clusters. The answer is relatively simple: RTF is designed to
_compose_ with other tools rather than embed them directly. By setting things up this way we make it
possible for end users to leverage the tooling they are already familiar with alongside RTF rather
than being forced to pick from a limited set of options that we happed to have added support for.

## Pay for what you use

So if that's what RTF is / is not in terms of design, what does it look like to actually use?

We here at Runtime Readiness are _big_ fans of the Unix Philosophy (as stated by Doug McIlroy):

> Write programs that do one thing and do it well.<br/> Write programs to work together.<br/> Write
> programs to handle text streams, because that is a universal interface.

In our case, as with most developer tooling, "text streams" is better replaced with structured data
in the form of YAML and JSON but the spirit of the philosophy remains.

We certainly require a variety of machinery to automate spinning up the services under test and
pulling together all of the data and configuration required to run representative test scenarios.
But we always make sure that each piece of what we write is usable independently (as far as
possible) and that if you don't want or need something, you shouldn't have to even know that it
exists.

To achieve that, our tooling is written as a collection of layers that you are free to pick and
chose from as you see fit. Each individual piece will have its own requirements and expectations for
you to be able to make use of it, but you should quickly see that many of those are shared so you
get quite a lot of bang for your buck!

## So, what's actually on offer?

RTF and the additional tooling surrounding it is organised as a set of three distinct layers that
work together to allow you, the user, to determine how and where to focus your time and resources.
With each option there are a different set of trade offs involved which mainly concern whether you
would prefer for everything to be as hands off as possible or if you would like to be able to
configure things just the way you like them. As you might imagine, the further down the layers you
go, the more control you have but also fewer guarantees.

### Layer 3: Custom user facing functionality built using RTF

At one end of the scale we have custom applications built using RTF as a foundation such as the
[Router release validation tests][0]. Here RTF is very much an implementation detail from the user's
perspective and the focus is entirely on addressing a particular testing need. If the application in
question is one you want or need to make use of, it will have accompanying documentation to guide
you through the process. If you are interested in developing your own testing capabilities on top of
RTF then these existing applications are a useful place to start in order to get some ideas about
what is possible.

### Layer 2: Configurable test plans, environments, scenarios & providers

If you would prefer something a little more custom then the next layer down lets you make use of the
same building blocks we use ourselves for writing our own _Layer 3_ applications. Here you can find
general purpose Scenario and Environment configurations such as those available in the [lib][1]
directory of the `rtf-morgue` repo.

> Soon you will also be able to define and re-use custom providers once [RR-351][2] is complete.

The focus of _Layer 2_ is to build and re-use higher level abstractions that address common use
cases which require coordinating elements that make use of shared semantics or configuration.

As far as possible, anything that is done by one of our _Layer 3_ applications is also made
available as part of _Layer 2_ for direct use as well. If you find yourself needing to do something
that isn't yet supported by our APIs or libraries, please do reach out to us in the
`#proj-runtime-testing-framework` channel in Slack to let us know! Chances are that other users
would like the same functionality and it might make sense to support it natively from our side.

### Layer 1: RTF core

The final layer is RTF itself. Here we provide the `rtf` CLI which supports a number of
[built-in providers][3] that serve as a minimal foundation for writing test plans. At this layer you
have maximum flexibility to implement what you need but at the cost of minimal guarantees around how
you have composed together the different elements that make up your test plan.

We should emphasise that "minimal" here is in comparison to what is available from Layers 2 and 3:
RTF provides a variety of debugging commands and rich error output to help you write and debug your
test plans. What it _doesn't_ provide out of the box is a way for ensuring that you've set up your
providers and test scripts in a self consistent way.

## Next steps

Now that you've read a little of the "why" its time to dig into the "how". Unsurprisingly, the
[Getting started][4] section is where we recommend you start first if your goal is to learn about
how RTF works and how you can work with pre-existing test plans. If you want to dig more into the
framework itself, then the [Test plans][5] and [framework][6] sections are more likely what you are
after.

Happy testing!

[0]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/1814036490/Router+Release+Validation
[1]: https://github.com/apollographql/rtf-morgue/tree/main/lib
[2]: https://apollographql.atlassian.net/browse/RR-351
[3]: ./framework/file-providers.md
[4]: ./guides/index.md
[5]: ./guides/test-plans/index.md
[6]: ./framework/index.md

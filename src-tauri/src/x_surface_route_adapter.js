// Exact, presentation-only adapter around an ORIGINAL public route factory.
// No downloaded-source evaluation, module-cache/React-state access, account
// changes, player calls or extra source API requests.
(() => {
  'use strict';
  const config=__ATSUMI_X_SOURCE_COMPAT__;
  if(window!==window.top||location.protocol!=='https:'||!['x.com','www.x.com'].includes(location.hostname)||location.port||!config.nonce||config.feed==='preview')return;
  const key='__atsumiXRouteAdapter';if(Object.hasOwn(window,key))return;
  const name='[A-Za-z_$][A-Za-z0-9_$]*';
  const escaped=value=>value.replace(/[$]/g,'\\$');
  function imported(code,alias){
    const header=new RegExp(`^(?:function(?: ${name})?|${name}|[0-9]+)?\\((${name}),(${name}),(${name})\\)\\{`).exec(code.trim());
    if(!header)return null;
    const expression=new RegExp(`(?:^|[;,{}])(?:var |let |const )?${escaped(alias)}=${escaped(header[3])}\\(([1-9][0-9]{0,8})\\)`, 'g');
    const found=[...code.matchAll(expression)];return found.length===1?Number(found[0][1]):null;
  }
  function shape(code){
    if(typeof code!=='string'||code.length>4*1024*1024||code.split('"immersiveMediaViewer"').length!==2)return null;
    const marker=',featureSwitchAnyOf:["explore_relaunch_enable_immersive_web","explore_relaunch_enable_video_carousels_immersive_viewer"],match:';
    if(code.split(marker).length!==2)return null;
    const at=code.indexOf(marker),before=code.slice(Math.max(0,at-512),at);
    const entry=new RegExp(`(${name})=(${name})=>\\(0,(${name})\\.jsx\\)\\((${name})\\.A,\\{\\.\\.\\.(${name}),component:(${name})$`).exec(before);
    if(!entry||entry[2]!==entry[5]||!code.slice(at+marker.length).startsWith(`${entry[2]}.match,withoutScribeImpression:!0})`))return null;
    // The route suffix lives in a different public module. Verify this
    // factory's actual registration of the SAME wrapper for a status route.
    const registration=code.slice(Math.max(0,code.indexOf('"immersiveMediaViewer"')-512),code.indexOf('"immersiveMediaViewer"'));
    if(!registration.includes(`component:${entry[1]},disableAnimation:!0,exact:!0,modalSize:`)||!registration.includes('/status/')||!registration.includes('full,path:'))return null;
    const modalCall=new RegExp(`\\(0,${escaped(entry[3])}\\.jsx\\)\\((${name})\\.A,\\{component:${escaped(entry[1])},disableAnimation:!0,exact:!0,modalSize:`).exec(registration);
    if(!modalCall)return null;
    const jsx=imported(code,entry[3]),gate=imported(code,entry[4]),modal=imported(code,modalCall[1]);
    return jsx&&gate&&modal&&new Set([jsx,gate,modal]).size===3?{jsx,gate,modal}:null;
  }
  function layoutShape(code){
    if(typeof code!=='string'||code.length>4*1024*1024||code.split('ImmersiveMediaViewerScreen:()=>').length!==2||code.split('testID:"modalContainer"').length!==2)return null;
    const size=new RegExp(`\\{width:(${name})\\}=(${name})\\.A\\.get\\("screen"\\)`).exec(code);
    if(!size||code.split(`${size[2]}.A.get("screen")`).length!==2)return null;
    const width=escaped(size[1]);
    const redirect=new RegExp(`${width}>=(${name})\\.A\\.theme\\.breakpoints\\.large\\)return void (${name})\\.push\\(\`/\\$\\{(${name})\\}/status/\\$\\{(${name})\\}\`\\);(${name})\\(${width}>=\\1\\.A\\.theme\\.breakpoints\\.large\\),\\2\\.replace\\(`);
    const branch=redirect.exec(code);if(!branch)return null;
    const dimensions=imported(code,size[2]),theme=imported(code,branch[1]);return dimensions&&theme&&dimensions!==theme?{dimensions,theme}:null;
  }
  function adapt(nonce,packet,kind='viewer'){
    if(nonce!==config.nonce||!Array.isArray(packet)||!packet[1]||typeof packet[1]!=='object')return null;
    if(!['viewer','layout'].includes(kind))return null;
    const modules=packet[1],keys=Object.keys(modules);if(keys.length>512)return null;
    const matches=[];
    for(const id of keys){if(typeof modules[id]!=='function')continue;const code=Function.prototype.toString.call(modules[id]),metadata=kind==='layout'?layoutShape(code):shape(code);if(metadata)matches.push({id,metadata});}
    if(matches.length!==1)return null;
    const {id,metadata}=matches[0],original=modules[id];
    const changed=packet.slice();changed[1]={...modules};
    changed[1][id]=function(...args){
      const require=args[2];if(typeof require!=='function')return Reflect.apply(original,this,args);
      let gate=null,modal=null,jsxSeen=false,gateSeen=false,modalSeen=false,largeBreakpoint=null;
      const alwaysModal=()=>true;
      const enable=()=>{if(jsxSeen&&gateSeen&&modalSeen)window.__atsumiXSurface?.registerSourceMediaViewer?.(config.nonce);};
      const imported=new Proxy(require,{apply(target,receiver,values){
        const module=Reflect.apply(target,receiver,values);
        if(kind==='layout'){
          if(values[0]===metadata.theme){
            const large=module?.A?.theme?.breakpoints?.large;
            if(typeof large==='number'&&Number.isFinite(large)&&large>1)largeBreakpoint=large;
            else window.__atsumiXSurface?.registerSourceMediaViewer?.(config.nonce,false);
            return module;
          }
          if(values[0]!==metadata.dimensions)return module;
          const dimensions=module?.A;
          if(!dimensions||typeof dimensions.get!=='function'){window.__atsumiXSurface?.registerSourceMediaViewer?.(config.nonce,false);return module;}
          const scoped=Object.create(module),viewport=Object.create(dimensions);
          Object.defineProperty(viewport,'get',{value:function(measure,...rest){
            if(measure==='screen'&&largeBreakpoint!==null&&window.__atsumiXSurface?.sourceMediaViewerAvailable?.()===true){
              const original=Reflect.apply(dimensions.get,dimensions,['window',...rest]);
              if(original&&typeof original.width==='number'&&Number.isFinite(original.width))return {...original,width:Math.min(original.width,largeBreakpoint-1)};
            }
            return Reflect.apply(dimensions.get,dimensions,[measure,...rest]);
          }});
          Object.defineProperty(scoped,'A',{value:viewport});return scoped;
        }
        if(values[0]===metadata.modal){
          const component=module?.A,defaults=component?.defaultProps;
          if(component&&(typeof component==='function'||typeof component==='object')
            &&typeof defaults?.shouldAlwaysDisplayModal==='function'&&typeof defaults?.shouldRenderAsModal==='function'){
            modal=component;modalSeen=true;enable();
          }else window.__atsumiXSurface?.registerSourceMediaViewer?.(config.nonce,false);
          return module;
        }
        if(values[0]===metadata.gate){
          const component=module?.A;
          if(component&&(typeof component==='function'||typeof component==='object')){gate=component;gateSeen=true;enable();}
          return module;
        }
        if(values[0]!==metadata.jsx||!module||typeof module.jsx!=='function')return module;
        jsxSeen=true;enable();
        const scoped=Object.create(module);
        Object.defineProperty(scoped,'jsx',{value:function(component,props,...rest){
          if(modal&&component===modal&&rest[0]==='immersiveMediaViewer'
            &&window.__atsumiXSurface?.sourceMediaViewerAvailable?.()===true){
            // The original router otherwise replaces its inline tree with a
            // modal at its responsive breakpoint, recreating paused players.
            // Keep this one source route in its original modal at every width.
            if(props&&typeof props.component==='function'&&props.disableAnimation===true&&props.exact===true
              &&props.modalSize==='full'&&props.shouldAlwaysDisplayModal===undefined
              &&Object.keys(props).length===5){
              return Reflect.apply(module.jsx,module,[component,{...props,shouldAlwaysDisplayModal:alwaysModal},...rest]);
            }
            window.__atsumiXSurface?.registerSourceMediaViewer?.(config.nonce,false);
          }
          if(gate&&component===gate&&window.__atsumiXSurface?.sourceMediaViewerAvailable?.()===true
            &&props&&props.component&&props.withoutScribeImpression===true&&Array.isArray(props.featureSwitchAnyOf)
            &&props.featureSwitchAnyOf.length===2&&props.featureSwitchAnyOf[0]==='explore_relaunch_enable_immersive_web'
            &&props.featureSwitchAnyOf[1]==='explore_relaunch_enable_video_carousels_immersive_viewer'){
            const {component:viewer,featureSwitchAnyOf:_,...sourceProps}=props;
            return Reflect.apply(module.jsx,module,[viewer,sourceProps,...rest]);
          }
          return Reflect.apply(module.jsx,module,[component,props,...rest]);
        }});
        return scoped;
      }});
      const supplied=args.slice();supplied[2]=imported;
      try{return Reflect.apply(original,this,supplied);}catch(error){window.__atsumiXSurface?.registerSourceMediaViewer?.(config.nonce,false);throw error;}
    };
    return changed;
  }
  Object.defineProperty(window,key,{value:Object.freeze({adapt,...(config.diagnostics===true?{shape,layoutShape}:{})})});
})();
